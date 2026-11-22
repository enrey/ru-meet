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

    pub(crate) fn correct(
        &mut self,
        utterance: &mut TimedUtterance,
        samples: &[f32],
        boundaries: &[f64],
    ) -> Vec<TermCorrection> {
        correct(utterance, samples, boundaries, |audio| {
            self.recognize(audio)
        })
    }
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
    samples: &[f32],
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
            continue;
        };
        let a = ((start - 2.0).max(utterance.1 / 1000.0) * 16000.0) as usize;
        let b = (((end + 2.0).min(utterance.2 / 1000.0) * 16000.0) as usize).min(samples.len());
        let Some(audio) = samples
            .get(a..b)
            .filter(|x| (1600..=128000).contains(&x.len()))
        else {
            continue;
        };
        let Some(candidate) = recognize(audio) else {
            continue;
        };
        if let Some((range, replacement)) = aligned_replacement(&utterance.0, &candidate, chunk) {
            if range.end <= protected
                && time_for(&utterance.3, &range)
                    .is_some_and(|(start, end)| !boundaries.iter().any(|t| start < *t && *t < end))
            {
                protected = range.start;
                edits.push(patch(utterance, range, replacement, "multilingual_220m"));
            }
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
    let left = *base.get(hit.checked_sub(1)?)?;
    if is_mixed(left.as_str()) {
        return None;
    }
    let folded_left = left.as_str().to_lowercase();
    for stop in hit + 1..=(hit + 3).min(base.len().saturating_sub(1)) {
        let right = base[stop];
        if is_mixed(right.as_str()) {
            continue;
        }
        // A missing right anchor is not permission to eat readable Cyrillic
        // words. Expansion is reserved for split foreign-name fragments.
        if base[hit + 1..stop]
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
        let range = base[hit].start()..base[stop - 1].end();
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

/// Subtract a strong delayed copy of the synchronized system signal. Keep the
/// residual so overlapping near-end speech is still passed to the VAD.
pub(crate) fn suppress_echo(microphone: &mut [f32], system: &[f32]) {
    const WINDOW: usize = 16000;
    const MAX_DELAY: usize = 4000;
    for start in (0..microphone.len()).step_by(WINDOW) {
        let end = (start + WINDOW).min(microphone.len()).min(system.len());
        let from = start.max(MAX_DELAY);
        if end.saturating_sub(from) < 3200 {
            continue;
        }
        let estimate = |lag: usize| {
            let (mut dot, mut mic, mut reference) = (0.0f64, 0.0, 0.0);
            for i in (from..end).step_by(16) {
                let a = microphone[i] as f64;
                let b = system[i - lag] as f64;
                dot += a * b;
                mic += a * a;
                reference += b * b;
            }
            let strength = dot.abs() / (mic * reference).sqrt().max(1e-8);
            (strength, dot / reference.max(1e-8))
        };
        let best = |lags: &mut dyn Iterator<Item = usize>| {
            lags.map(|lag| (lag, estimate(lag)))
                .max_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
        };
        let Some((coarse, _)) = best(&mut (0..=MAX_DELAY).step_by(8)) else {
            continue;
        };
        let Some((lag, (correlation, gain))) =
            best(&mut (coarse.saturating_sub(7)..=(coarse + 7).min(MAX_DELAY)))
        else {
            continue;
        };
        if correlation >= 0.65 && gain.abs() <= 2.0 {
            for i in start.max(lag)..end {
                let fade = ((i - start).min(end - i - 1) as f32 / 160.0).min(1.0);
                microphone[i] -= fade * gain as f32 * system[i - lag];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_correction_does_not_call_second_model() {
        let mut part = ("Сказал «сoрри», потом Docker.".into(), 0.0, 2000.0, vec![]);
        let changes = correct(&mut part, &[], &[], |_| panic!("unexpected model call"));
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
        assert!(correct(&mut part, &audio, &[1.0], |_| Some(
            "про конфлюенс завтра".into()
        ))
        .is_empty());
        assert_eq!(part.0, before.0);
        let changes = correct(&mut part, &audio, &[], |_| {
            Some("про конфлюенс завтра".into())
        });
        assert_eq!(part.0, "про конфлюенс завтра");
        assert_eq!(changes[0].start, Some(0.5));
        assert_eq!(part.3.len(), 3);
        assert_eq!(&part.0[part.3[2].byte_start..], "завтра");
        assert_eq!(part.3[2].start, 2.0);
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
    fn subtracts_aligned_echo_and_retains_independent_speech() {
        let mut random = 185u32;
        let system: Vec<_> = (0..32000)
            .map(|_| {
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                ((random >> 16) as f32 / 65535.0 - 0.5) * 0.3
            })
            .collect();
        let near: Vec<_> = (0..32000)
            .map(|i| (i as f32 * 0.031).sin() * 0.02)
            .collect();
        let mut mic: Vec<_> = near
            .iter()
            .enumerate()
            .map(|(i, &own)| own + i.checked_sub(160).map_or(0.0, |idx| system[idx] * 0.6))
            .collect();
        suppress_echo(&mut mic, &system);
        let mean_error = mic[8000..12000]
            .iter()
            .zip(&near[8000..12000])
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            / 4000.0;
        assert!(mean_error < 0.004, "{mean_error}");
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
