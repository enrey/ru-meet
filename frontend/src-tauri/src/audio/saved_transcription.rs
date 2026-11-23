use super::{common::TimedUtterance, word_timing::WordTiming};
use crate::gigaam_onnx::{GigaAMModel, Quantization, TranscribeOptions};
use anyhow::{bail, Result};
use futures_util::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};
use tauri::{AppHandle, Runtime};
use tokio::io::AsyncWriteExt;

const BASE_URL: &str = "https://huggingface.co/istupakov/gigaam-multilingual-ctc-onnx/resolve/458860e1983aef670dd9795fb6af603c82767d5d";
const FILES: [(&str, &str, u64, &str); 2] = [
    (
        "multilingual_ctc.int8.onnx",
        "model.int8.onnx",
        224_762_204,
        "e08e27ae5669b39f0c378fae101bbbb9a80505f74f9b66719c309bf5b894a480",
    ),
    (
        "multilingual_vocab.txt",
        "vocab.txt",
        393,
        "4d130287892e1099fedfb3f93c4b4cf8a263151158801680b28977d1be4133f4",
    ),
];
static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LETTERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{L}+").unwrap());
static WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]+").unwrap());
static LATIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{Latin}").unwrap());
static CYRILLIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{Cyrillic}").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingSource {
    Mix,
    Microphone,
    System,
}

impl RecordingSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Mix => "mix",
            Self::Microphone => "microphone",
            Self::System => "system",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "mix" => Some(Self::Mix),
            "microphone" => Some(Self::Microphone),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermCorrection {
    pub original: String,
    pub replacement: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub method: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PostRecording {
    pub source_tracks: bool,
    pub speaker_boundaries: Vec<f64>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub(crate) struct Provenance {
    pub source: Option<RecordingSource>,
    pub corrections: Vec<TermCorrection>,
}

fn model_dir(models: &Path) -> PathBuf {
    models.join("gigaam").join("multilingual-220m-int8")
}

fn model_available(dir: &Path) -> bool {
    FILES.iter().all(|(_, name, size, _)| {
        dir.join(name)
            .metadata()
            .is_ok_and(|m| m.is_file() && m.len() == *size)
    })
}

#[tauri::command]
pub(crate) fn term_correction_model_available<R: Runtime>(
    app: AppHandle<R>,
) -> Result<bool, String> {
    let data = crate::portable::app_data_dir(&app).map_err(|err| err.to_string())?;
    Ok(model_available(&model_dir(&data.join("models"))))
}

#[tauri::command]
pub(crate) async fn install_term_correction_model<R: Runtime>(
    app: AppHandle<R>,
) -> Result<(), String> {
    let data = crate::portable::app_data_dir(&app).map_err(|err| err.to_string())?;
    install_model(&model_dir(&data.join("models")))
        .await
        .map_err(|err| err.to_string())
}

async fn install_model(dir: &Path) -> Result<()> {
    let _install = INSTALL.lock().await;
    if model_available(dir) {
        return Ok(());
    }
    let parent = dir.parent().unwrap();
    tokio::fs::create_dir_all(parent).await?;
    let staging = tempfile::tempdir_in(parent)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()?;
    for (remote, name, size, expected) in FILES {
        let mut stream = client
            .get(format!("{BASE_URL}/{remote}"))
            .send()
            .await?
            .error_for_status()?
            .bytes_stream();
        let mut hash = Sha256::new();
        let mut file = tokio::fs::File::create(staging.path().join(name)).await?;
        let mut written = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            written += chunk.len() as u64;
            if written > size {
                bail!("Unexpected size for {remote}");
            }
            hash.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        if written != size || format!("{:x}", hash.finalize()) != expected {
            bail!("Model verification failed for {remote}");
        }
    }
    tokio::fs::create_dir_all(dir).await?;
    // Publish the large file last.
    for (_, name, _, _) in FILES.into_iter().rev() {
        let dst = dir.join(name);
        if dst.exists() {
            tokio::fs::remove_file(&dst).await?;
        }
        tokio::fs::rename(staging.path().join(name), dst).await?;
    }
    Ok(())
}

pub(crate) struct Corrector {
    directory: Option<PathBuf>,
    model: Option<GigaAMModel>,
}

impl Corrector {
    pub(crate) fn new(models: &Path) -> Self {
        let dir = model_dir(models);
        Self {
            directory: model_available(&dir).then_some(dir),
            model: None,
        }
    }

    fn recognize(&mut self, samples: &[f32]) -> Option<String> {
        if self.model.is_none() {
            let dir = self.directory.take()?;
            match GigaAMModel::load(&dir, &Quantization::Int8) {
                Ok(model) => self.model = Some(model),
                Err(err) => log::warn!("Could not load term correction model: {err:#}"),
            }
        }
        self.model
            .as_mut()?
            .transcribe(samples, &TranscribeOptions::default())
            .map(|result| result.text)
            .map_err(|error| log::warn!("Term correction inference failed: {error:#}"))
            .ok()
    }

    /// `mix` is the audio the transcript was built from; `tracks` are the
    /// recording's source tracks (microphone, system) on the same timeline,
    /// empty for older or imported recordings.
    pub(crate) fn correct(
        &mut self,
        utterance: &mut TimedUtterance,
        mix: &[f32],
        tracks: &[Track],
        boundaries: &[f64],
    ) -> Vec<TermCorrection> {
        correct(utterance, mix, tracks, boundaries, |audio| {
            self.recognize(audio)
        })
    }
}

/// A source track and its typical speech level, so tracks recorded at very
/// different levels (loopback after the output volume) compare fairly.
pub(crate) struct Track {
    pub samples: Vec<f32>,
    level: f32,
}

impl Track {
    pub(crate) fn new(samples: Vec<f32>) -> Self {
        let level = speech_level(&samples);
        Self { samples, level }
    }

    /// Window loudness relative to this track's own speech.
    fn prominence(&self, range: Range<usize>) -> f32 {
        let window =
            &self.samples[range.start.min(self.samples.len())..range.end.min(self.samples.len())];
        if window.is_empty() || self.level <= 0.0 {
            return 0.0;
        }
        rms(window) / self.level
    }
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// 90th percentile of 100 ms RMS over the audible part of a track.
fn speech_level(samples: &[f32]) -> f32 {
    let mut levels: Vec<f32> = samples
        .chunks(1600)
        .map(rms)
        .filter(|level| *level > 1e-4)
        .collect();
    if levels.is_empty() {
        return 0.0;
    }
    levels.sort_by(f32::total_cmp);
    levels[levels.len() * 9 / 10]
}

/// Whether any word needs the second model (cheap check before decoding tracks).
pub(crate) fn has_mixed_word(text: &str) -> bool {
    LETTERS.find_iter(text).any(|m| is_mixed(m.as_str()))
}

fn is_mixed(word: &str) -> bool {
    LATIN.is_match(word) && CYRILLIC.is_match(word)
}

fn time_for(words: &[WordTiming], range: &Range<usize>) -> Option<(f64, f64)> {
    let first = words
        .partition_point(|w| w.byte_start <= range.start)
        .checked_sub(1)?;
    let last = words
        .partition_point(|w| w.byte_start < range.end)
        .checked_sub(1)?;
    let (start, end) = (words[first].start, words[last].end);
    (start.is_finite() && end.is_finite() && start >= 0.0 && end > start).then_some((start, end))
}

fn patch(
    utterance: &mut TimedUtterance,
    range: Range<usize>,
    replacement: &str,
    method: &str,
) -> TermCorrection {
    let times = time_for(&utterance.3, &range);
    let correction = TermCorrection {
        original: utterance.0[range.clone()].to_owned(),
        replacement: replacement.to_owned(),
        start: times.map(|t| t.0),
        end: times.map(|t| t.1),
        method: method.to_owned(),
    };
    let delta = replacement.len() as isize - range.len() as isize;
    let last = utterance.3.partition_point(|w| w.byte_start < range.end);
    for word in &mut utterance.3[last..] {
        word.byte_start = word.byte_start.saturating_add_signed(delta);
    }
    if method != "letters" {
        if let Some((start, end)) = times {
            let first = utterance.3.partition_point(|w| w.byte_start <= range.start) - 1;
            let first_byte = utterance.3[first].byte_start;
            let tokens: Vec<_> = WORDS.find_iter(replacement).collect();
            let step = (end - start) / tokens.len() as f64;
            utterance.3.splice(
                first..last,
                tokens.into_iter().enumerate().map(|(i, token)| WordTiming {
                    byte_start: if i == 0 {
                        first_byte
                    } else {
                        range.start + token.start()
                    },
                    start: start + step * i as f64,
                    end: (start + step * (i + 1) as f64).min(end),
                }),
            );
        }
    }
    utterance.0.replace_range(range, replacement);
    correction
}

fn correct(
    utterance: &mut TimedUtterance,
    mix: &[f32],
    tracks: &[Track],
    boundaries: &[f64],
    mut recognize: impl FnMut(&[f32]) -> Option<String>,
) -> Vec<TermCorrection> {
    let chunks: Vec<_> = LETTERS
        .find_iter(&utterance.0)
        .filter(|m| is_mixed(m.as_str()))
        .map(|m| m.range())
        .collect();
    let mut edits = Vec::new();
    // Offsets to the left of a replacement are still valid.
    let mut protected = utterance.0.len();
    for chunk in chunks.into_iter().rev() {
        if chunk.end > protected {
            continue;
        }
        let word = &utterance.0[chunk.clone()];
        if word.to_lowercase() == "сoрри" {
            let replacement = word.replace('o', "о").replace('O', "О");
            protected = chunk.start;
            edits.push(patch(utterance, chunk, &replacement, "letters"));
            continue;
        }
        let Some((start, end)) = time_for(&utterance.3, &chunk) else {
            log::info!("Term correction skipped a word: it has no timing");
            continue;
        };
        let a = ((start - 2.0).max(utterance.1 / 1000.0) * 16000.0) as usize;
        let b = ((end + 2.0).min(utterance.2 / 1000.0) * 16000.0) as usize;
        // The track carrying this voice first, then the other one, then the
        // mix; the first reading that fits the surrounding words wins.
        let word_range = (start * 16000.0) as usize..(end * 16000.0) as usize;
        let mut sources: Vec<&[f32]> = Vec::with_capacity(tracks.len() + 1);
        let mut ordered: Vec<&Track> = tracks.iter().collect();
        ordered.sort_by(|x, y| {
            y.prominence(word_range.clone())
                .total_cmp(&x.prominence(word_range.clone()))
        });
        sources.extend(ordered.into_iter().map(|track| track.samples.as_slice()));
        sources.push(mix);
        let mut read = 0;
        let mut fitted = false;
        let mut corrected = false;
        for source in sources {
            let Some(audio) = source
                .get(a..b.min(source.len()))
                .filter(|x| (1600..=128000).contains(&x.len()))
            else {
                continue;
            };
            let Some(candidate) = recognize(audio) else {
                continue;
            };
            read += 1;
            if let Some((range, replacement)) =
                aligned_replacement(&utterance.0, &candidate, chunk.clone())
            {
                fitted = true;
                if range.end <= protected
                    && time_for(&utterance.3, &range).is_some_and(|(start, end)| {
                        !boundaries.iter().any(|t| start < *t && *t < end)
                    })
                {
                    protected = range.start;
                    edits.push(patch(utterance, range, replacement, "multilingual_220m"));
                    corrected = true;
                    break;
                }
            }
        }
        if !corrected {
            let at = format!("{}:{:02}", start as u64 / 60, start as u64 % 60);
            let reason = if read == 0 {
                "the second model gave no reading"
            } else if fitted {
                "the replacement would cross a speaker change or an earlier correction"
            } else {
                "no reading matched the neighbouring words"
            };
            log::info!("Term correction left the word at {at} ({start:.1} s) as recognized: {reason} ({read} readings)");
        }
    }
    edits.reverse();
    edits
}

fn aligned_replacement<'a>(
    text: &str,
    candidate: &'a str,
    trigger: Range<usize>,
) -> Option<(Range<usize>, &'a str)> {
    let base: Vec<_> = WORDS.find_iter(text).collect();
    let alt: Vec<_> = WORDS.find_iter(candidate).collect();
    let hit = base
        .iter()
        .position(|m| m.start() <= trigger.start && m.end() >= trigger.end)?;
    // A hyphenated compound ("IT-tкоде") is one unit: anchors are taken
    // outside it and it is replaced whole, never by halves.
    let joined = |a: usize, b: usize| &text[base[a].end()..base[b].start()] == "-";
    let mut first = hit;
    while first > 0 && joined(first - 1, first) {
        first -= 1;
    }
    let mut last = hit;
    while last + 1 < base.len() && joined(last, last + 1) {
        last += 1;
    }
    let left = *base.get(first.checked_sub(1)?)?;
    if is_mixed(left.as_str()) {
        return None;
    }
    let folded_left = left.as_str().to_lowercase();
    for stop in last + 1..=(last + 3).min(base.len().saturating_sub(1)) {
        let right = base[stop];
        if is_mixed(right.as_str()) {
            continue;
        }
        // A missing right anchor is not permission to eat readable Cyrillic
        // words. Expansion is reserved for split foreign-name fragments.
        if base[last + 1..stop]
            .iter()
            .any(|m| !LATIN.is_match(m.as_str()) || CYRILLIC.is_match(m.as_str()))
        {
            break;
        }
        let folded_right = right.as_str().to_lowercase();
        let matches: Vec<_> = (0..alt.len())
            .flat_map(|i| ((i + 2)..(i + 6).min(alt.len())).map(move |j| (i, j)))
            .filter(|&(i, j)| {
                alt[i].as_str().to_lowercase() == folded_left
                    && alt[j].as_str().to_lowercase() == folded_right
            })
            .collect();
        let [(i, j)] = matches.as_slice() else {
            continue;
        };
        let range = base[first].start()..base[stop - 1].end();
        if [
            &text[left.end()..range.start],
            &text[range.end..right.start()],
        ]
        .iter()
        .any(|gap| gap.contains('-'))
        {
            continue;
        }
        let replacement = &candidate[alt[*i + 1].start()..alt[*j - 1].end()];
        if replacement != &text[range.clone()]
            && LETTERS.is_match(replacement)
            && !LETTERS.find_iter(replacement).any(|m| is_mixed(m.as_str()))
        {
            return Some((range, replacement));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_correction_does_not_call_second_model() {
        let mut part = ("Сказал «сoрри», потом Docker.".into(), 0.0, 2000.0, vec![]);
        let changes = correct(&mut part, &[], &[], &[], |_| {
            panic!("unexpected model call")
        });
        assert_eq!(part.0, "Сказал «сорри», потом Docker.");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn preserves_neighbors_and_repairs_word_offsets() {
        let mut part: TimedUtterance = ("про коnflinc tito G завтра".into(), 0.0, 3000.0, vec![]);
        part.3 = WORDS
            .find_iter(&part.0)
            .enumerate()
            .map(|(i, m)| WordTiming {
                byte_start: m.start(),
                start: i as f64 * 0.5,
                end: i as f64 * 0.5 + 0.4,
            })
            .collect();
        let audio = vec![0.0; 48000];
        let before = part.clone();
        assert!(correct(&mut part, &audio, &[], &[1.0], |_| Some(
            "про конфлюенс завтра".into()
        ))
        .is_empty());
        assert_eq!(part.0, before.0);
        let changes = correct(&mut part, &audio, &[], &[], |_| {
            Some("про конфлюенс завтра".into())
        });
        assert_eq!(part.0, "про конфлюенс завтра");
        assert_eq!(changes[0].start, Some(0.5));
        assert_eq!(part.3.len(), 3);
        assert_eq!(&part.0[part.3[2].byte_start..], "завтра");
        assert_eq!(part.3[2].start, 2.0);
    }

    #[test]
    fn hyphenated_compound_is_replaced_whole() {
        let text = "всё в облачном IT-tкоде. Угу.";
        let range = text.find("tкоде").unwrap()..text.find("tкоде").unwrap() + "tкоде".len();
        let (replaced, replacement) =
            aligned_replacement(text, "всё в облачном ITTooCode угу", range).unwrap();
        assert_eq!(&text[replaced], "IT-tкоде");
        assert_eq!(replacement, "ITTooCode");
    }

    /// "про коnflinc завтра" with word timings 0.5 s apart.
    fn suspicious_row() -> TimedUtterance {
        let mut part: TimedUtterance = ("про коnflinc завтра".into(), 0.0, 3000.0, vec![]);
        part.3 = WORDS
            .find_iter(&part.0)
            .enumerate()
            .map(|(i, m)| WordTiming {
                byte_start: m.start(),
                start: i as f64 * 0.5,
                end: i as f64 * 0.5 + 0.4,
            })
            .collect();
        part
    }

    #[test]
    fn track_carrying_the_voice_is_tried_first_then_the_rest() {
        // Microphone is quiet overall but loud at the word: relative level
        // wins even though the system track is louder in absolute terms.
        let mut microphone = vec![0.01; 48000];
        microphone[8000..11200].fill(0.05);
        let microphone = Track::new(microphone);
        let system = Track::new(vec![0.2; 48000]);
        let mix = vec![0.3; 48000];
        let mut calls = Vec::new();
        let mut part = suspicious_row();
        let changes = correct(&mut part, &mix, &[system, microphone], &[], |audio| {
            calls.push(audio[0]);
            // Only the second source gives a reading that fits the row.
            (calls.len() == 2).then(|| "про конфлюенс завтра".into())
        });
        assert_eq!(calls, vec![0.01, 0.2]);
        assert_eq!(part.0, "про конфлюенс завтра");
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn without_tracks_the_mix_alone_is_used() {
        let mut calls = 0;
        let mut part = suspicious_row();
        correct(&mut part, &vec![0.1; 48000], &[], &[], |_| {
            calls += 1;
            None
        });
        assert_eq!(calls, 1);
        assert_eq!(part.0, "про коnflinc завтра");
    }

    #[test]
    fn missing_neighbor_does_not_consume_readable_context() {
        let source = "про миting скажи завтра";
        let range = LETTERS
            .find_iter(source)
            .find(|m| is_mixed(m.as_str()))
            .unwrap()
            .range();
        assert!(aligned_replacement(source, "про митинг завтра", range).is_none());
    }

    #[test]
    #[ignore = "uses the local AAC and pinned 220M ONNX fixtures"]
    fn native_multilingual_model_recognizes_audio() {
        use super::super::decoder::{decode_recording_stream, RecordingStream};
        let fixtures =
            PathBuf::from(std::env::var_os("RU_MEET_SAVED_TRANSCRIPTION_FIXTURE").unwrap());
        let runtime =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries/onnxruntime/onnxruntime.dll");
        crate::init_worker_onnx(Some(&runtime)).unwrap();
        let samples =
            decode_recording_stream(&fixtures.join("separate.mp4"), RecordingStream::Microphone)
                .unwrap()
                .unwrap()
                .to_whisper_format();
        let text = Corrector::new(&fixtures.join("models"))
            .recognize(&samples)
            .unwrap();
        assert!(LETTERS.is_match(&text));
    }
}
