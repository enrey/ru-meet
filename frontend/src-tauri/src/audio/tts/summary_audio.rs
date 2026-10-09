//! The summary read aloud, prepared ahead of time into an audio file next to
//! the meeting.
//!
//! Synthesis is slower than real time on weaker machines, so instead of
//! speaking while the user listens, the whole summary is synthesized in the
//! background (right after it is generated, or when Play is pressed) and
//! stitched into one file. The player then treats it like the recording:
//! instant start, exact duration, ordinary seeking.
//!
//! Next to the audio sits a JSON file with the time of every spoken word, for
//! the karaoke highlight, and a digest of the text it was made from: a summary
//! edited since then no longer matches and is read again. Sentences are cached
//! one by one, so after an edit only the changed ones are synthesized.

use super::commands::{can_speak, data_root, engine, settings, speaking_variant};
use super::text;
use anyhow::{anyhow, bail, Context, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

const FILE_STEM: &str = "summary-speech";
const TIMINGS_FILE: &str = "summary-speech.json";
/// Bumped when the stitching or the timings change, so old files are redone.
const FORMAT_VERSION: u32 = 2;
const SENTENCE_GAP_SECONDS: f32 = 0.35;
const PARAGRAPH_GAP_SECONDS: f32 = 0.8;

/// One spoken word and when it is heard. Times are estimated: the model gives
/// no alignment, so each sentence's speech is shared between its words by
/// length, with pauses after punctuation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpokenWord {
    pub text: String,
    pub start: f32,
    pub end: f32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Timings {
    version: u32,
    digest: String,
    /// File name of the audio, in the same folder.
    audio: String,
    duration: f32,
    words: Vec<SpokenWord>,
}

/// What the meeting page's player can do with the summary reading.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SummaryAudio {
    /// Reading aloud is off, or the model is not installed.
    Unavailable,
    /// There is no summary to read.
    NoSummary,
    /// Not prepared yet; `stale` when a reading of an older summary exists.
    NotPrepared {
        stale: bool,
    },
    Preparing {
        done: usize,
        total: usize,
    },
    Failed {
        message: String,
    },
    Ready {
        path: String,
        duration: f32,
        words: Vec<SpokenWord>,
    },
}

enum Job {
    Running {
        generation: u64,
        done: usize,
        total: usize,
    },
    Failed(String),
}

/// Preparations by meeting. A newer request for the same meeting replaces the
/// entry, and the older job notices at its next sentence and gives up.
static JOBS: Lazy<Mutex<HashMap<String, Job>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
/// One synthesis at a time: they all share the GPU.
static WORKER: Mutex<()> = Mutex::new(());

fn is_current(meeting_id: &str, generation: u64) -> bool {
    matches!(
        JOBS.lock().ok().and_then(|jobs| match jobs.get(meeting_id) {
            Some(Job::Running { generation: running, .. }) => Some(*running),
            _ => None,
        }),
        Some(running) if running == generation
    )
}

fn set_progress(meeting_id: &str, generation: u64, done: usize, total: usize) {
    if let Ok(mut jobs) = JOBS.lock() {
        if let Some(Job::Running {
            generation: running,
            done: d,
            total: t,
        }) = jobs.get_mut(meeting_id)
        {
            if *running == generation {
                *d = done;
                *t = total;
            }
        }
    }
}

fn pool<R: Runtime>(app: &AppHandle<R>) -> sqlx::SqlitePool {
    app.state::<crate::state::AppState>()
        .db_manager
        .pool()
        .clone()
}

/// The summary as the reading speaks it.
async fn summary_markdown(pool: &sqlx::SqlitePool, meeting_id: &str) -> Result<Option<String>> {
    let process =
        crate::database::repositories::summary::SummaryProcessesRepository::get_summary_data(
            pool, meeting_id,
        )
        .await
        .context("Could not load the summary")?;
    let Some(raw) = process.and_then(|process| process.result) else {
        return Ok(None);
    };
    let value: serde_json::Value =
        serde_json::from_str(&raw).context("The stored summary is not JSON")?;
    Ok(crate::summary::export::summary_to_markdown(&value)
        .filter(|markdown| !markdown.trim().is_empty()))
}

/// Where the reading of a meeting is kept: its recording folder, or a folder
/// of its own under the app data for meetings without one.
async fn speech_folder<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<PathBuf> {
    let folder: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .context("Could not look up the meeting folder")?
            .flatten();
    folder_for(app, folder.as_deref(), meeting_id)
}

fn folder_for<R: Runtime>(
    app: &AppHandle<R>,
    folder: Option<&str>,
    meeting_id: &str,
) -> Result<PathBuf> {
    if let Some(folder) = folder.map(PathBuf::from).filter(|folder| folder.is_dir()) {
        return Ok(folder);
    }
    let digest = format!("{:x}", md5::compute(meeting_id.as_bytes()));
    Ok(data_root(app)?.join("summary-speech").join(digest))
}

/// A meeting's reading as the library lists it.
pub enum ListedReading {
    /// Not prepared, outdated, or reading aloud is off.
    None,
    Preparing,
    Ready,
    Failed(String),
}

/// The reading's state for the meeting list, from what the list query already
/// loaded: no database access, one small file read. An outdated reading whose
/// sentences are all cached is put together again on the spot.
pub fn listed_reading<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    folder: Option<&str>,
    markdown: Option<&str>,
) -> ListedReading {
    let job = JOBS
        .lock()
        .ok()
        .and_then(|jobs| match jobs.get(meeting_id) {
            Some(Job::Running { .. }) => Some(ListedReading::Preparing),
            Some(Job::Failed(message)) => Some(ListedReading::Failed(message.clone())),
            None => None,
        });
    if let Some(state) = job {
        return state;
    }
    let Some(markdown) = markdown.filter(|markdown| !markdown.trim().is_empty()) else {
        return ListedReading::None;
    };
    let Ok(folder) = folder_for(app, folder, meeting_id) else {
        return ListedReading::None;
    };
    // Cheapest check first: most meetings were never read aloud.
    let Some(timings) = read_timings(&folder) else {
        return ListedReading::None;
    };
    if !can_speak(app) {
        return ListedReading::None;
    }
    let chunks = text::summary_to_chunks(markdown);
    let variant = speaking_variant(app);
    if timings.digest == digest(variant.id(), &chunks) && folder.join(&timings.audio).is_file() {
        return ListedReading::Ready;
    }
    let cached = !chunks.is_empty()
        && chunks.iter().all(|(_, chunk)| {
            sentence_path(app, variant.id(), chunk).is_ok_and(|path| path.is_file())
        });
    if cached {
        prepare(app, meeting_id.to_string());
        return ListedReading::Preparing;
    }
    ListedReading::None
}

fn digest(variant: &str, chunks: &[(usize, String)]) -> String {
    let mut source = format!("v{FORMAT_VERSION}\n{variant}");
    for (paragraph, chunk) in chunks {
        source.push_str(&format!("\n{paragraph}\t{chunk}"));
    }
    format!("{:x}", md5::compute(source.as_bytes()))
}

/// A synthesized sentence, cached by the checkpoint and its text.
fn sentence_path<R: Runtime>(app: &AppHandle<R>, variant: &str, sentence: &str) -> Result<PathBuf> {
    let name = format!("{:x}.wav", md5::compute(sentence.as_bytes()));
    Ok(data_root(app)?.join("tts-cache").join(variant).join(name))
}

fn read_timings(folder: &Path) -> Option<Timings> {
    let raw = std::fs::read_to_string(folder.join(TIMINGS_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// The state of a meeting's reading, granting the webview access to its audio
/// when it is ready.
#[tauri::command]
pub async fn tts_summary_audio<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<SummaryAudio, String> {
    summary_audio(&app, &meeting_id)
        .await
        .map_err(|error| error.to_string())
}

async fn summary_audio<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<SummaryAudio> {
    if !can_speak(app) {
        return Ok(SummaryAudio::Unavailable);
    }
    if let Some(job) = JOBS
        .lock()
        .map_err(|_| anyhow!("The TTS job lock is poisoned"))?
        .get(meeting_id)
    {
        return Ok(match job {
            Job::Running { done, total, .. } => SummaryAudio::Preparing {
                done: *done,
                total: *total,
            },
            Job::Failed(message) => SummaryAudio::Failed {
                message: message.clone(),
            },
        });
    }

    let pool = pool(app);
    let Some(markdown) = summary_markdown(&pool, meeting_id).await? else {
        return Ok(SummaryAudio::NoSummary);
    };
    let chunks = text::summary_to_chunks(&markdown);
    if chunks.is_empty() {
        return Ok(SummaryAudio::NoSummary);
    }
    let variant = speaking_variant(app);
    let wanted = digest(variant.id(), &chunks);
    let folder = speech_folder(app, &pool, meeting_id).await?;
    let timings = read_timings(&folder);
    let audio = timings.as_ref().map(|timings| folder.join(&timings.audio));
    let (Some(timings), Some(audio)) = (timings, audio) else {
        return Ok(SummaryAudio::NotPrepared { stale: false });
    };
    if timings.digest != wanted || !audio.is_file() {
        // Every sentence is already synthesized (say, only the stitching
        // changed): putting the reading together again costs nothing.
        let cached = chunks.iter().all(|(_, chunk)| {
            sentence_path(app, variant.id(), chunk).is_ok_and(|path| path.is_file())
        });
        if cached {
            prepare(app, meeting_id.to_string());
            return Ok(SummaryAudio::Preparing {
                done: 0,
                total: chunks.len(),
            });
        }
        return Ok(SummaryAudio::NotPrepared { stale: true });
    }
    app.asset_protocol_scope().allow_file(&audio)?;
    Ok(SummaryAudio::Ready {
        path: audio.to_string_lossy().to_string(),
        duration: timings.duration,
        words: timings.words,
    })
}

/// Start preparing a meeting's reading in the background. Progress arrives as
/// `summary-audio-progress`, the end as `summary-audio-ready` or
/// `summary-audio-failed`, all with the meeting's id.
#[tauri::command]
pub fn tts_prepare_summary_audio<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
) -> Result<(), String> {
    if !can_speak(&app) {
        return Err("Reading summaries aloud is turned off or not installed".to_string());
    }
    prepare(&app, meeting_id);
    Ok(())
}

/// After a summary is generated or saved: prepare its reading right away when
/// the user asked for that.
pub fn prepare_if_automatic<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    if settings().auto_prepare && can_speak(app) {
        prepare(app, meeting_id.to_string());
    }
}

fn prepare<R: Runtime>(app: &AppHandle<R>, meeting_id: String) {
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut jobs) = JOBS.lock() {
        jobs.insert(
            meeting_id.clone(),
            Job::Running {
                generation,
                done: 0,
                total: 0,
            },
        );
    }
    let _ = app.emit(
        "summary-audio-progress",
        serde_json::json!({ "meetingId": meeting_id, "done": 0, "total": 0 }),
    );

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = async {
            let pool = pool(&app);
            let markdown = summary_markdown(&pool, &meeting_id)
                .await?
                .ok_or_else(|| anyhow!("There is no summary to read"))?;
            let folder = speech_folder(&app, &pool, &meeting_id).await?;
            let worker_app = app.clone();
            let worker_meeting = meeting_id.clone();
            tokio::task::spawn_blocking(move || {
                build(&worker_app, &worker_meeting, generation, &markdown, &folder)
            })
            .await
            .map_err(|error| anyhow!("The speech job stopped: {error}"))?
        }
        .await;

        let current = is_current(&meeting_id, generation);
        match outcome {
            Ok(true) if current => {
                if let Ok(mut jobs) = JOBS.lock() {
                    jobs.remove(&meeting_id);
                }
                let _ = app.emit(
                    "summary-audio-ready",
                    serde_json::json!({ "meetingId": meeting_id }),
                );
            }
            Err(error) if current => {
                log::warn!("Summary reading for {meeting_id} failed: {error:#}");
                let message = error.to_string();
                if let Ok(mut jobs) = JOBS.lock() {
                    jobs.insert(meeting_id.clone(), Job::Failed(message.clone()));
                }
                let _ = app.emit(
                    "summary-audio-failed",
                    serde_json::json!({ "meetingId": meeting_id, "message": message }),
                );
            }
            // Superseded by a newer request, which reports for itself.
            _ => {}
        }
    });
}

/// Synthesize, stitch and save. Returns false when a newer request for the
/// same meeting took over.
fn build<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    generation: u64,
    markdown: &str,
    folder: &Path,
) -> Result<bool> {
    let _worker = WORKER
        .lock()
        .map_err(|_| anyhow!("The speech worker lock is poisoned"))?;
    let chunks = text::summary_to_chunks(markdown);
    if chunks.is_empty() {
        bail!("There is nothing to read in this summary");
    }
    let variant = speaking_variant(app);
    let wanted = digest(variant.id(), &chunks);
    if read_timings(folder)
        .is_some_and(|timings| timings.digest == wanted && folder.join(&timings.audio).is_file())
    {
        return Ok(true);
    }

    let started = std::time::Instant::now();
    let total = chunks.len();
    let mut sentences: Vec<PathBuf> = Vec::with_capacity(total);
    let mut synthesized = 0;
    for (index, (_, chunk)) in chunks.iter().enumerate() {
        if !is_current(meeting_id, generation) {
            return Ok(false);
        }
        wait_while_recording(meeting_id, generation);
        let path = sentence_path(app, variant.id(), chunk)?;
        if !path.is_file() {
            let partial = path.with_extension("partial.wav");
            engine(app)?.synthesize(chunk, &partial)?;
            std::fs::rename(&partial, &path)?;
            synthesized += 1;
        }
        sentences.push(path);
        set_progress(meeting_id, generation, index + 1, total);
        let _ = app.emit(
            "summary-audio-progress",
            serde_json::json!({ "meetingId": meeting_id, "done": index + 1, "total": total }),
        );
    }

    std::fs::create_dir_all(folder)?;
    let (samples, rate, words) = stitch(&chunks, &sentences)?;
    let duration = samples.len() as f32 / rate as f32;
    let audio = save_audio(folder, &samples, rate)?;
    let timings = Timings {
        version: FORMAT_VERSION,
        digest: wanted,
        audio,
        duration,
        words,
    };
    let partial = folder.join(format!("{TIMINGS_FILE}.partial"));
    std::fs::write(&partial, serde_json::to_vec(&timings)?)?;
    std::fs::rename(&partial, folder.join(TIMINGS_FILE))?;
    log::info!(
        "Summary reading for {meeting_id}: {total} sentences ({synthesized} new), {duration:.1}s of audio, in {:.1}s",
        started.elapsed().as_secs_f32()
    );
    Ok(true)
}

/// A recording needs the machine more than a reading does.
fn wait_while_recording(meeting_id: &str, generation: u64) {
    let mut announced = false;
    while tauri::async_runtime::block_on(crate::audio::recording_commands::is_recording()) {
        if !is_current(meeting_id, generation) {
            return;
        }
        if !announced {
            log::info!("Summary reading for {meeting_id} waits for the recording to finish");
            announced = true;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

/// Samples of a mono 16-bit PCM WAV, which is what `llama-tts` writes.
fn read_wav(path: &Path) -> Result<(Vec<i16>, u32)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("{} is not a WAV file", path.display());
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            format = Some((
                u16::from_le_bytes(body[0..2].try_into()?),
                u16::from_le_bytes(body[2..4].try_into()?),
                u32::from_le_bytes(body[4..8].try_into()?),
                u16::from_le_bytes(body[14..16].try_into()?),
            ));
        } else if id == b"data" {
            let Some((1, 1, rate, 16)) = format else {
                bail!("{} is not mono 16-bit PCM", path.display());
            };
            let samples = body
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            return Ok((samples, rate));
        }
        at += 8 + size + (size & 1);
    }
    bail!("{} has no audio data", path.display())
}

/// The audible part of a sentence, in samples: synthesis pads both ends with
/// near-silence that no word should be highlighted over.
fn speech_span(samples: &[i16], rate: u32) -> (usize, usize) {
    const THRESHOLD: i32 = 600;
    let frame = (rate as usize / 100).max(1);
    let loud = |frame_samples: &[i16]| {
        frame_samples
            .iter()
            .any(|sample| (*sample as i32).abs() > THRESHOLD)
    };
    let frames: Vec<&[i16]> = samples.chunks(frame).collect();
    let first = frames.iter().position(|samples| loud(samples));
    let last = frames.iter().rposition(|samples| loud(samples));
    match (first, last) {
        (Some(first), Some(last)) => (first * frame, ((last + 1) * frame).min(samples.len())),
        _ => (0, samples.len()),
    }
}

/// Share `start..end` seconds between the words of `sentence` by length,
/// leaving a pause after punctuation.
fn word_times(sentence: &str, start: f32, end: f32) -> Vec<SpokenWord> {
    let mut words: Vec<(String, f32, f32)> = Vec::new(); // text, weight, pause after
    for token in sentence.split_whitespace() {
        let letters = token.chars().filter(|c| c.is_alphanumeric()).count();
        let pause = match token.chars().last() {
            Some('.' | '!' | '?' | '…') => 4.0,
            Some(',' | ';' | ':') => 2.5,
            _ => 0.0,
        };
        if letters == 0 {
            // A dash or a stray mark: only a pause.
            if let Some(last) = words.last_mut() {
                last.2 += 2.5;
            }
            continue;
        }
        words.push((token.to_string(), letters as f32 + 1.0, pause));
    }
    let total: f32 = words.iter().map(|(_, weight, pause)| weight + pause).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let unit = (end - start).max(0.0) / total;
    let mut cursor = start;
    words
        .into_iter()
        .map(|(text, weight, pause)| {
            let word = SpokenWord {
                text,
                start: cursor,
                end: cursor + weight * unit,
            };
            cursor += (weight + pause) * unit;
            word
        })
        .collect()
}

/// One track of all sentences, with short pauses between sentences and longer
/// ones between paragraphs, and the time of every word on it.
fn stitch(
    chunks: &[(usize, String)],
    sentences: &[PathBuf],
) -> Result<(Vec<i16>, u32, Vec<SpokenWord>)> {
    let mut samples: Vec<i16> = Vec::new();
    let mut rate = 0;
    let mut words = Vec::new();
    for (index, ((paragraph, sentence), path)) in chunks.iter().zip(sentences).enumerate() {
        let (audio, sentence_rate) = read_wav(path)?;
        if rate == 0 {
            rate = sentence_rate;
        } else if rate != sentence_rate {
            bail!("Sentences were synthesized at different sample rates");
        }
        if index > 0 {
            let gap = if chunks[index - 1].0 == *paragraph {
                SENTENCE_GAP_SECONDS
            } else {
                PARAGRAPH_GAP_SECONDS
            };
            samples.extend(std::iter::repeat(0).take((gap * rate as f32) as usize));
        }
        let offset = samples.len();
        let (from, to) = speech_span(&audio, rate);
        words.extend(word_times(
            sentence,
            (offset + from) as f32 / rate as f32,
            (offset + to) as f32 / rate as f32,
        ));
        samples.extend_from_slice(&audio);
    }
    Ok((samples, rate, words))
}

fn write_wav(path: &Path, samples: &[i16], rate: u32) -> Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Save the track as AAC through the bundled ffmpeg - a tenth of the WAV's
/// size - or as the WAV itself when ffmpeg cannot. Returns the file name.
fn save_audio(folder: &Path, samples: &[i16], rate: u32) -> Result<String> {
    let wav = folder.join(format!("{FILE_STEM}.partial.wav"));
    write_wav(&wav, samples, rate)?;

    let m4a_name = format!("{FILE_STEM}.m4a");
    let wav_name = format!("{FILE_STEM}.wav");
    let encoded = folder.join(format!("{FILE_STEM}.partial.m4a"));
    let converted = crate::audio::ffmpeg::find_ffmpeg_path().is_some_and(|ffmpeg| {
        let mut command = std::process::Command::new(ffmpeg);
        command
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&wav)
            .args(["-c:a", "aac", "-b:a", "64k"])
            .arg(&encoded);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        match command.output() {
            Ok(output) if output.status.success() && encoded.is_file() => true,
            Ok(output) => {
                log::warn!(
                    "ffmpeg could not encode the summary reading: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                false
            }
            Err(error) => {
                log::warn!("ffmpeg could not run for the summary reading: {error}");
                false
            }
        }
    });

    let (kept, name, other) = if converted {
        let _ = std::fs::remove_file(&wav);
        (encoded, m4a_name, wav_name)
    } else {
        let _ = std::fs::remove_file(&encoded);
        (wav, wav_name, m4a_name)
    };
    std::fs::rename(&kept, folder.join(&name))?;
    let _ = std::fs::remove_file(folder.join(other));
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_share_the_speech_and_pause_after_punctuation() {
        let words = word_times("Да, нет.", 1.0, 2.0);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Да,");
        assert!((words[0].start - 1.0).abs() < 1e-6);
        // The comma's pause separates the words.
        assert!(words[1].start > words[0].end + 0.1);
        assert!(words[1].end <= 2.0 + 1e-6);
    }

    #[test]
    fn a_dash_is_a_pause_not_a_word() {
        let words = word_times("Итог — принято", 0.0, 1.0);
        assert_eq!(
            words
                .iter()
                .map(|word| word.text.as_str())
                .collect::<Vec<_>>(),
            ["Итог", "принято"]
        );
    }

    #[test]
    fn silence_around_speech_is_trimmed() {
        let mut samples = vec![0i16; 1000];
        samples.extend(vec![5000i16; 1000]);
        samples.extend(vec![0i16; 1000]);
        let (from, to) = speech_span(&samples, 10_000);
        assert_eq!((from, to), (1000, 2000));
    }

    #[test]
    fn a_written_wav_reads_back() {
        let folder =
            std::env::temp_dir().join(format!("summary-audio-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("test.wav");
        write_wav(&path, &[1, -2, 3], 24_000).unwrap();
        assert_eq!(read_wav(&path).unwrap(), (vec![1, -2, 3], 24_000));
        let _ = std::fs::remove_dir_all(folder);
    }
}
