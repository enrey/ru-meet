//! Direct GigaAM v3 ONNX inference, bypassing the `transcribe-rs` crate.
//!
//! `transcribe-rs` hard-pins `ort = "=2.0.0-rc.12"`, which has a confirmed
//! deadlock bug (fixed upstream in `ort` 2.0.0-rc.13: "Don't deadlock when
//! `load-dynamic` fails.") in the dylib-loading path we depend on for
//! Parakeet/VAD. Since GigaAM's own ONNX graph is a single CTC encoder with
//! no exotic pre/post-processing, it's cheaper to talk to `ort` directly here
//! (matching the crate version used everywhere else in this app) than to
//! carry a second, pinned-to-a-buggy-version copy of the runtime.
//!
//! The mel-spectrogram, CTC greedy decode, vocab loading, and SentencePiece
//! detokenization below are ported from `transcribe-rs` (MIT licensed):
//! <https://github.com/cjpais/transcribe-rs/blob/main/src/onnx/gigaam/mod.rs>
//! and its sibling `features/mel.rs`, `decode/{ctc,tokens,sentencepiece}.rs>`.

use anyhow::{anyhow, Result};
use ndarray::Array2;
use ort::inputs;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;
use std::path::Path;

/// Preferred precision for ONNX model loading. Selects which model file
/// variant to load; falls back to FP32 (`model.onnx`) if the requested
/// variant isn't present on disk.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Quantization {
    #[default]
    FP32,
    FP16,
    Int8,
    Int4,
}

fn resolve_model_path(dir: &Path, name: &str, quantization: &Quantization) -> std::path::PathBuf {
    let suffix = match quantization {
        Quantization::FP32 => None,
        Quantization::FP16 => Some("fp16"),
        Quantization::Int8 => Some("int8"),
        Quantization::Int4 => Some("int4"),
    };

    if let Some(suffix) = suffix {
        let path = dir.join(format!("{}.{}.onnx", name, suffix));
        if path.exists() {
            return path;
        }
        log::warn!(
            "{} model not found at {}, falling back to {}.onnx",
            suffix,
            path.display(),
            name
        );
    }

    dir.join(format!("{}.onnx", name))
}

/// Options for transcription. GigaAM v3 is a single-language (Russian),
/// non-streaming acoustic model, so it has nothing to configure per-call;
/// this only exists so call sites can stay written against a options-style
/// API like the other engines.
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions;

#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
}

// ---- Mel spectrogram (ported from transcribe-rs `features/mel.rs`) --------

struct MelConfig {
    sample_rate: u32,
    num_mels: usize,
    n_fft: usize,
    hop_length: usize,
    f_min: f32,
    f_max: Option<f32>,
}

/// Standard mel spectrogram (GigaAM-style): windowed STFT + mel filterbank + log.
/// Returns `[num_frames, num_mels]`.
fn compute_mel(samples: &[f32], config: &MelConfig) -> Array2<f32> {
    let sr = config.sample_rate as f32;
    let f_max = config.f_max.unwrap_or(sr / 2.0);
    let n_fft = config.n_fft;
    let hop_length = config.hop_length;

    if samples.len() < n_fft {
        return Array2::zeros((0, config.num_mels));
    }

    let n_frames = (samples.len() - n_fft) / hop_length + 1;
    let freq_bins = n_fft / 2 + 1;

    let window = make_hann_window(n_fft);
    let filterbank = mel_filterbank(config.num_mels, n_fft, sr, config.f_min, f_max);

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n_fft);

    // Compute STFT power spectrogram [freq_bins, n_frames]
    let mut power_spec = Array2::<f32>::zeros((freq_bins, n_frames));

    for frame_idx in 0..n_frames {
        let start = frame_idx * hop_length;
        let mut fft_buf: Vec<Complex<f32>> = (0..n_fft)
            .map(|i| Complex::new(samples[start + i] * window[i], 0.0))
            .collect();

        fft.process(&mut fft_buf);

        for (bin, val) in fft_buf.iter().enumerate().take(freq_bins) {
            power_spec[[bin, frame_idx]] = val.norm_sqr();
        }
    }

    // Apply mel filterbank: [num_mels, freq_bins] @ [freq_bins, n_frames] = [num_mels, n_frames]
    let mel = filterbank.dot(&power_spec);

    // Log scaling with clamping, then transpose to [n_frames, num_mels]
    mel.mapv(|v| v.clamp(1e-9, 1e9).ln()).t().to_owned()
}

fn make_hann_window(length: usize) -> Vec<f32> {
    (0..length)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / length as f32).cos()))
        .collect()
}

fn mel_filterbank(
    num_mels: usize,
    fft_size: usize,
    sample_rate: f32,
    low_freq: f32,
    high_freq: f32,
) -> Array2<f32> {
    let num_fft_bins = fft_size / 2 + 1;

    let mel_low = hz_to_mel(low_freq);
    let mel_high = hz_to_mel(high_freq);

    let num_points = num_mels + 2;
    let mel_points: Vec<f32> = (0..num_points)
        .map(|i| mel_low + (mel_high - mel_low) * i as f32 / (num_points - 1) as f32)
        .collect();

    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    let bin_points: Vec<f32> = hz_points
        .iter()
        .map(|&f| f * fft_size as f32 / sample_rate)
        .collect();

    let mut banks = Array2::zeros((num_mels, num_fft_bins));

    for m in 0..num_mels {
        let left = bin_points[m];
        let center = bin_points[m + 1];
        let right = bin_points[m + 2];

        for k in 0..num_fft_bins {
            let kf = k as f32;
            if kf > left && kf < center {
                banks[[m, k]] = (kf - left) / (center - left);
            } else if kf >= center && kf < right {
                banks[[m, k]] = (right - kf) / (right - center);
            }
        }
    }

    banks
}

fn hz_to_mel(hz: f32) -> f32 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * ((mel / 1127.0).exp() - 1.0)
}

// ---- Vocab loading (ported from transcribe-rs `decode/tokens.rs`) --------

/// Load a vocabulary file where each line is `token id`.
///
/// Returns a Vec indexed by token ID, and the blank token index.
/// Replaces `▁` (U+2581) with space in token strings.
fn load_vocab(path: &Path) -> Result<(Vec<String>, Option<i32>)> {
    let content = std::fs::read_to_string(path)?;

    let mut max_id = 0;
    let mut tokens_with_ids: Vec<(String, usize)> = Vec::new();
    let mut blank_idx: Option<i32> = None;

    for line in content.lines() {
        let parts: Vec<&str> = line.trim_end().split(' ').collect();
        if parts.len() >= 2 {
            let token = parts[0].to_string();
            if let Ok(id) = parts[1].parse::<usize>() {
                if token == "<blk>" {
                    blank_idx = Some(id as i32);
                }
                tokens_with_ids.push((token, id));
                max_id = max_id.max(id);
            }
        }
    }

    let mut vocab = vec![String::new(); max_id + 1];
    for (token, id) in tokens_with_ids {
        vocab[id] = token.replace('\u{2581}', " ");
    }

    log::info!("Loaded {} vocab tokens from {:?}", vocab.len(), path);
    Ok((vocab, blank_idx))
}

// ---- CTC greedy decode (ported from transcribe-rs `decode/ctc.rs`) -------

/// For each time step, selects the token with highest logit. Skips blank
/// tokens and consecutive repeated tokens.
fn ctc_greedy_decode(logits: &ndarray::ArrayView3<f32>, num_frames: usize, blank_id: i64) -> Vec<i64> {
    let vocab_size = logits.shape()[2];
    let mut tokens = Vec::new();
    let mut prev_id: i64 = -1;

    for t in 0..num_frames {
        let mut max_val = f32::NEG_INFINITY;
        let mut max_id: i64 = 0;
        for v in 0..vocab_size {
            let val = logits[[0, t, v]];
            if val > max_val {
                max_val = val;
                max_id = v as i64;
            }
        }

        if max_id != blank_id && max_id != prev_id {
            tokens.push(max_id);
        }
        prev_id = max_id;
    }

    tokens
}

// ---- SentencePiece detokenization (ported from `decode/sentencepiece.rs`) -

/// Convert a sequence of SentencePiece tokens to readable text.
fn sentencepiece_to_text(tokens: &[&str]) -> String {
    let mut text = String::new();
    for &token in tokens {
        text.push_str(&token.replace('\u{2581}', " "));
    }
    let text = text.trim().to_string();
    text.replace(" '", "'")
}

// ---- Model ------------------------------------------------------------

pub struct GigaAMModel {
    session: Session,
    mel_config: MelConfig,
    vocab: Vec<String>,
    blank_idx: i64,
}

impl GigaAMModel {
    pub fn load(model_dir: &Path, quantization: &Quantization) -> Result<Self> {
        let model_path = resolve_model_path(model_dir, "model", quantization);
        let vocab_path = model_dir.join("vocab.txt");

        if !model_path.exists() {
            return Err(anyhow!("GigaAM model not found at {}", model_path.display()));
        }
        if !vocab_path.exists() {
            return Err(anyhow!("GigaAM vocab not found at {}", vocab_path.display()));
        }

        log::info!("Loading GigaAM model from {:?}...", model_path);
        let session = Session::builder()
            .map_err(|error| anyhow!("failed to create GigaAM ONNX session: {error}"))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|error| anyhow!("failed to configure GigaAM ONNX session: {error}"))?
            .commit_from_file(&model_path)
            .map_err(|error| anyhow!("failed to load GigaAM model {}: {error}", model_path.display()))?;

        let (vocab, blank_idx) = load_vocab(&vocab_path)?;
        let blank_idx = blank_idx.unwrap_or(vocab.len() as i32) as i64;

        log::info!(
            "Loaded vocabulary with {} tokens, blank_idx={}",
            vocab.len(),
            blank_idx
        );

        let mel_config = MelConfig {
            sample_rate: 16000,
            num_mels: 64,
            n_fft: 320,
            hop_length: 160,
            f_min: 0.0,
            f_max: Some(8000.0),
        };

        Ok(Self {
            session,
            mel_config,
            vocab,
            blank_idx,
        })
    }

    pub fn transcribe(
        &mut self,
        samples: &[f32],
        _options: &TranscribeOptions,
    ) -> Result<TranscriptionResult> {
        if samples.len() < self.mel_config.n_fft {
            return Ok(TranscriptionResult { text: String::new() });
        }

        // 1. Compute mel spectrogram [frames, mels]
        let mel = compute_mel(samples, &self.mel_config);
        let time_steps = mel.shape()[0];

        // 2. Prepare input tensors: features [1, n_mels, time], feature_lengths [1].
        // ONNX model expects [1, mels, time], so transpose then add batch dim.
        let features = mel.t().to_owned().insert_axis(ndarray::Axis(0)); // [1, 64, T]
        let features_dyn = features.into_dyn();
        let feature_lengths = ndarray::arr1(&[time_steps as i64]).into_dyn();

        // 3. Run ONNX forward pass
        let t_features = TensorRef::from_array_view(features_dyn.view())?;
        let t_lengths = TensorRef::from_array_view(feature_lengths.view())?;
        let outputs = self.session.run(inputs! {
            "features" => t_features,
            "feature_lengths" => t_lengths,
        })?;

        // 4. Extract log_probs [1, T', vocab_size]
        let log_probs = outputs[0].try_extract_array::<f32>()?;
        let log_probs = log_probs.to_owned().into_dimensionality::<ndarray::Ix3>()?;

        // 5. CTC greedy decode
        let num_frames = log_probs.shape()[1];
        let token_ids = ctc_greedy_decode(&log_probs.view(), num_frames, self.blank_idx);

        // 6. Convert token IDs to text
        let tokens: Vec<&str> = token_ids
            .iter()
            .filter_map(|&id| {
                let idx = id as usize;
                if idx < self.vocab.len() {
                    let token = self.vocab[idx].as_str();
                    if token == "<unk>" {
                        None
                    } else {
                        Some(token)
                    }
                } else {
                    None
                }
            })
            .collect();

        let text = sentencepiece_to_text(&tokens);

        Ok(TranscriptionResult { text })
    }
}
