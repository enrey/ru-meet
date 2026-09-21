use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use ndarray::{Array1, Array2, Array3, Ix3};
use ort::{inputs, session::Session, value::TensorRef};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;
use tauri::{AppHandle, Manager, Runtime};

// Global models directory path (set during app initialization), matching the
// same `app_data_dir()`-based location whisper/parakeet/gigaam use. Populated
// once via `set_models_directory` from `lib.rs`'s setup; a `cargo test`
// context that never calls it falls back to a legacy per-user directory (see
// `ensure_silero_model`) so tests keep working without a live Tauri app.
static MODELS_DIR: StdMutex<Option<PathBuf>> = StdMutex::new(None);

// Path to the model as bundled inside the app itself (see
// `build/silero_vad.rs` and the `binaries/silero` resource mapping in
// `tauri.windows.conf.json`) - Windows only for now. When this resolves to a
// verified file, `ensure_silero_model` loads it directly from there and
// never touches `MODELS_DIR` or the network at all; it's only a fallback
// location (macOS/Linux, or a build that didn't bundle it) that goes
// through `MODELS_DIR`/download.
static BUNDLED_MODEL_PATH: StdMutex<Option<PathBuf>> = StdMutex::new(None);

/// Initialize the models directory path using app_data_dir. Should be called
/// during app setup, alongside the other engines' `set_models_directory`.
pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(bundled) = app.path().resolve(
        "silero/silero_vad.onnx",
        tauri::path::BaseDirectory::Resource,
    ) {
        if verify_silero_model(&bundled) {
            info!("Using bundled Silero VAD model at {}", bundled.display());
            *BUNDLED_MODEL_PATH.lock().unwrap() = Some(bundled);
        }
    }

    let Ok(app_data_dir) = crate::portable::app_data_dir(app) else {
        log::error!("Failed to get app data dir for Silero VAD models directory");
        return;
    };
    let models_dir = app_data_dir.join("models");
    if let Err(error) = std::fs::create_dir_all(&models_dir) {
        log::error!("Failed to create Silero VAD models directory: {error}");
        return;
    }
    *MODELS_DIR.lock().unwrap() = Some(models_dir);
}

/// Silero VAD only operates at 16kHz; input is resampled to this rate, and every
/// sample count and timestamp inside this module is expressed in it.
const VAD_SAMPLE_RATE: u32 = 16000;
const SILERO_FRAME_SIZE: usize = 512;
// Pinned to a tagged release, not `master`: verified by direct inference test
// against real speech (JFK "Ask not..." sample) that the current `master` /
// v6.2.2 tag's silero_vad.onnx is degenerate - its "output" is essentially
// constant regardless of the "input" tensor's content or amplitude (max
// probability 0.0036 across an 11s speech clip that should score >0.9 during
// speech). v5.1.2's model responds correctly (mean 0.55, 60% of frames >=0.5
// on the same clip) and is what this is pinned to. Re-verify with a similar
// test before ever bumping this.
const SILERO_VAD_MODEL_URL: &str = "https://raw.githubusercontent.com/snakers4/silero-vad/v5.1.2/src/silero_vad/data/silero_vad.onnx";
const SILERO_VAD_MODEL_SHA256: &str =
    "2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f";
const SILERO_VAD_MODEL_SIZE: u64 = 2_327_524;

/// Minimal direct binding to the official Silero VAD ONNX graph.  Keeping this
/// here avoids a second `ort` version and makes the model contract explicit.
struct SileroVad {
    session: Session,
    state: Array3<f32>,
}

impl SileroVad {
    fn new() -> Result<Self> {
        let path = ensure_silero_model()?;
        let session = Session::builder()
            .map_err(|error| anyhow!("failed to create Silero VAD ONNX session: {error}"))?
            .commit_from_file(&path)
            .map_err(|error| {
                anyhow!(
                    "failed to load Silero VAD model {}: {error}",
                    path.display()
                )
            })?;
        Ok(Self {
            session,
            state: Array3::zeros((2, 1, 128)),
        })
    }

    fn probability(&mut self, samples: &[f32]) -> Result<f32> {
        debug_assert_eq!(samples.len(), SILERO_FRAME_SIZE);
        let audio = Array2::from_shape_vec((1, SILERO_FRAME_SIZE), samples.to_vec())?;
        let sample_rate = Array1::from_vec(vec![VAD_SAMPLE_RATE as i64]);
        let outputs = self.session.run(inputs![
            "input" => TensorRef::from_array_view(audio.view())?,
            "state" => TensorRef::from_array_view(self.state.view())?,
            "sr" => TensorRef::from_array_view(sample_rate.view())?,
        ])?;
        let probability = outputs
            .get("output")
            .ok_or_else(|| anyhow!("Silero VAD model did not return `output`"))?
            .try_extract_array::<f32>()?
            .iter()
            .next()
            .copied()
            .ok_or_else(|| anyhow!("Silero VAD model returned an empty `output` tensor"))?;
        self.state = outputs
            .get("stateN")
            .ok_or_else(|| anyhow!("Silero VAD model did not return `stateN`"))?
            .try_extract_array::<f32>()?
            .to_owned()
            .into_dimensionality::<Ix3>()?;
        Ok(probability)
    }
}

fn ensure_silero_model() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("MEETILY_SILERO_VAD_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(anyhow!(
            "MEETILY_SILERO_VAD_MODEL does not point to a file: {}",
            path.display()
        ));
    }

    // Load straight from the app's own install directory when it's bundled
    // there (Windows only, for now - see `BUNDLED_MODEL_PATH`'s doc comment).
    // No copying into `models_dir`, no network: this is the whole point of
    // shipping it in the installer.
    if let Some(bundled) = BUNDLED_MODEL_PATH.lock().unwrap().clone() {
        return Ok(bundled);
    }

    let models_dir = match MODELS_DIR.lock().unwrap().clone() {
        Some(dir) => dir,
        // No live Tauri app ever called `set_models_directory` (e.g. `cargo
        // test`) - fall back to the pre-unification location rather than
        // failing outright.
        None => dirs::data_dir()
            .ok_or_else(|| anyhow!("could not determine the application data directory"))?
            .join("Meetily")
            .join("models"),
    };
    let model_path = models_dir.join("silero_vad.onnx");
    if model_path.is_file() && verify_silero_model(&model_path) {
        return Ok(model_path);
    }
    if model_path.is_file() {
        warn!(
            "Cached Silero VAD model at {} failed checksum verification (stale/corrupt download); re-fetching",
            model_path.display()
        );
    }

    fs::create_dir_all(&models_dir)?;
    let temporary_path = models_dir.join("silero_vad.onnx.download");
    // `reqwest::blocking` spins up its own Tokio runtime internally and blocks
    // on it; doing that from a thread that's already inside a (multi-threaded)
    // Tokio runtime - which this function's callers all are, since VAD
    // processor creation happens on the recording pipeline's async task -
    // panics with "Cannot drop a runtime in a context where blocking is not
    // allowed" the moment that inner runtime is torn down. `block_in_place`
    // hands this thread off to blocking work without nesting a runtime.
    let bytes = tokio::task::block_in_place(|| -> Result<bytes::Bytes> {
        let response = reqwest::blocking::get(SILERO_VAD_MODEL_URL).map_err(|error| {
            anyhow!("failed to download the official Silero VAD model: {error}")
        })?;
        let response = response
            .error_for_status()
            .map_err(|error| anyhow!("official Silero VAD model download failed: {error}"))?;
        Ok(response.bytes()?)
    })?;
    if bytes.len() as u64 != SILERO_VAD_MODEL_SIZE {
        return Err(anyhow!(
            "official Silero VAD model download has size {}, expected {}",
            bytes.len(),
            SILERO_VAD_MODEL_SIZE
        ));
    }
    let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
    if actual_sha256 != SILERO_VAD_MODEL_SHA256 {
        return Err(anyhow!(
            "official Silero VAD model has SHA-256 {actual_sha256}, expected {SILERO_VAD_MODEL_SHA256}"
        ));
    }
    fs::write(&temporary_path, &bytes)?;
    fs::rename(&temporary_path, &model_path)?;
    info!(
        "Downloaded official Silero VAD model to {}",
        model_path.display()
    );
    Ok(model_path)
}

/// Verifies a cached model file's size and SHA-256 against the pinned
/// values. Returns `false` (never errors) so a stale/corrupt cache is simply
/// treated the same as a missing one and re-downloaded.
fn verify_silero_model(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if bytes.len() as u64 != SILERO_VAD_MODEL_SIZE {
        return false;
    }
    format!("{:x}", Sha256::digest(&bytes)) == SILERO_VAD_MODEL_SHA256
}

/// Represents a complete speech segment detected by VAD
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    pub samples: Vec<f32>,
    pub start_timestamp_ms: f64,
    pub end_timestamp_ms: f64,
    pub confidence: f32,
}

/// Processes audio in the official Silero 32ms frames but returns complete speech segments.
pub struct ContinuousVadProcessor {
    session: SileroVad,
    chunk_size: usize,
    sample_rate: u32,
    buffer: Vec<f32>,
    speech_segments: VecDeque<SpeechSegment>,
    current_speech: Vec<f32>,
    in_speech: bool,
    processed_samples: usize,
    speech_start_sample: usize,
    // State tracking for smart logging
    last_logged_state: bool,
    positive_speech_threshold: f32,
    negative_speech_threshold: f32,
    redemption_samples: usize,
    pre_speech_pad_samples: usize,
    post_speech_pad_samples: usize,
    min_speech_samples: usize,
    silence_samples: usize,
    pre_speech: VecDeque<f32>,
}

impl ContinuousVadProcessor {
    pub fn new(input_sample_rate: u32, redemption_time_ms: u32) -> Result<Self> {
        crate::ensure_onnx_runtime_available()?;

        debug!("Creating VAD session with: sample_rate={}Hz, redemption={}ms, min_speech={}ms, input_rate={}Hz",
               VAD_SAMPLE_RATE, redemption_time_ms, 250, input_sample_rate);
        let session = SileroVad::new()?;

        info!(
            "VAD processor created: input={}Hz, vad={}Hz, chunk_size={} samples",
            input_sample_rate, VAD_SAMPLE_RATE, SILERO_FRAME_SIZE
        );

        Ok(Self {
            session,
            chunk_size: SILERO_FRAME_SIZE,
            sample_rate: input_sample_rate, // Store input rate for resampling ratio in resample_to_16k()
            buffer: Vec::with_capacity(SILERO_FRAME_SIZE * 2),
            speech_segments: VecDeque::new(),
            current_speech: Vec::new(),
            in_speech: false,
            processed_samples: 0,
            speech_start_sample: 0,
            // Initialize state tracking
            last_logged_state: false,
            positive_speech_threshold: 0.50,
            negative_speech_threshold: 0.35,
            redemption_samples: redemption_time_ms as usize * VAD_SAMPLE_RATE as usize / 1000,
            pre_speech_pad_samples: 300 * VAD_SAMPLE_RATE as usize / 1000,
            post_speech_pad_samples: 400 * VAD_SAMPLE_RATE as usize / 1000,
            min_speech_samples: 250 * VAD_SAMPLE_RATE as usize / 1000,
            silence_samples: 0,
            pre_speech: VecDeque::with_capacity(300 * VAD_SAMPLE_RATE as usize / 1000),
        })
    }

    /// Process incoming audio samples and return any complete speech segments
    /// Handles resampling from input sample rate to 16kHz for VAD processing
    pub fn process_audio(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>> {
        // Resample to 16kHz if needed
        let resampled_audio = if self.sample_rate == 16000 {
            samples.to_vec()
        } else {
            self.resample_to_16k(samples)?
        };

        self.buffer.extend_from_slice(&resampled_audio);
        let mut completed_segments = Vec::new();

        // Process complete Silero frames (512 samples / 32ms at 16kHz).
        while self.buffer.len() >= self.chunk_size {
            let chunk: Vec<f32> = self.buffer.drain(..self.chunk_size).collect();
            self.process_chunk(&chunk, chunk.len())?;

            // Extract any completed speech segments
            while let Some(segment) = self.speech_segments.pop_front() {
                completed_segments.push(segment);
            }
        }

        Ok(completed_segments)
    }

    /// Improved resampling from input sample rate to 16kHz with anti-aliasing
    /// Uses linear interpolation and basic low-pass filtering for better quality
    fn resample_to_16k(&self, samples: &[f32]) -> Result<Vec<f32>> {
        if self.sample_rate == 16000 {
            return Ok(samples.to_vec());
        }

        // Calculate downsampling ratio
        let ratio = self.sample_rate as f64 / 16000.0;
        let output_len = (samples.len() as f64 / ratio) as usize;
        let mut resampled = Vec::with_capacity(output_len);

        // Apply simple low-pass filter before downsampling to reduce aliasing
        let cutoff_freq = 0.4; // Normalized frequency (0.4 * Nyquist)
        let mut filtered_samples = Vec::with_capacity(samples.len());

        // Simple moving average filter (basic low-pass)
        let filter_size =
            (self.sample_rate as f64 / (cutoff_freq * self.sample_rate as f64)) as usize;
        let filter_size = std::cmp::max(1, std::cmp::min(filter_size, 5)); // Limit filter size

        for i in 0..samples.len() {
            let start = if i >= filter_size { i - filter_size } else { 0 };
            let end = std::cmp::min(i + filter_size + 1, samples.len());
            let sum: f32 = samples[start..end].iter().sum();
            filtered_samples.push(sum / (end - start) as f32);
        }

        // Linear interpolation downsampling
        for i in 0..output_len {
            let source_pos = i as f64 * ratio;
            let source_index = source_pos as usize;
            let fraction = source_pos - source_index as f64;

            if source_index + 1 < filtered_samples.len() {
                // Linear interpolation
                let sample1 = filtered_samples[source_index];
                let sample2 = filtered_samples[source_index + 1];
                let interpolated = sample1 + (sample2 - sample1) * fraction as f32;
                resampled.push(interpolated);
            } else if source_index < filtered_samples.len() {
                resampled.push(filtered_samples[source_index]);
            }
        }

        debug!(
            "Resampled from {} samples ({}Hz) to {} samples (16kHz) with anti-aliasing",
            samples.len(),
            self.sample_rate,
            resampled.len()
        );

        Ok(resampled)
    }

    /// Flush any remaining audio and return final speech segments
    pub fn flush(&mut self) -> Result<Vec<SpeechSegment>> {
        debug!("VAD flush: in_speech={}, current_speech_len={}, buffer_len={}, speech_segments_queued={}",
              self.in_speech, self.current_speech.len(), self.buffer.len(), self.speech_segments.len());

        let mut completed_segments = Vec::new();
        // Preserve the real post-resampling endpoint before padding the final VAD frame.
        let real_end_sample = self.processed_samples + self.buffer.len();

        // Process any remaining buffered audio
        if !self.buffer.is_empty() {
            let remaining = self.buffer.clone();
            let remaining_len = remaining.len();
            self.buffer.clear();

            // Pad to chunk size if needed
            let mut padded_chunk = remaining;
            if padded_chunk.len() < self.chunk_size {
                padded_chunk.resize(self.chunk_size, 0.0);
            }

            self.process_chunk(&padded_chunk, remaining_len)?;
        }

        // Force end any ongoing speech
        if self.in_speech && !self.current_speech.is_empty() {
            let real_sample_count = real_end_sample
                .checked_sub(self.speech_start_sample)
                .filter(|count| *count > 0)
                .ok_or_else(|| {
                    anyhow!(
                        "VAD flush invariant violated: active speech interval [{}, {}) is empty or reversed",
                        self.speech_start_sample,
                        real_end_sample
                    )
                })?;
            if self.current_speech.len() < real_sample_count {
                return Err(anyhow!(
                    "VAD flush invariant violated: active speech buffer has {} samples, but [{}, {}) requires {}",
                    self.current_speech.len(),
                    self.speech_start_sample,
                    real_end_sample,
                    real_sample_count
                ));
            }
            let samples = self.current_speech[..real_sample_count].to_vec();
            let start_ms = (self.speech_start_sample as f64 / VAD_SAMPLE_RATE as f64) * 1000.0;
            let end_ms = (real_end_sample as f64 / VAD_SAMPLE_RATE as f64) * 1000.0;

            debug!(
                "VAD flush: Force-ending speech - start={}ms, end={}ms, duration={}ms, samples={}",
                start_ms,
                end_ms,
                end_ms - start_ms,
                samples.len()
            );

            let segment = SpeechSegment {
                samples,
                start_timestamp_ms: start_ms,
                end_timestamp_ms: end_ms,
                confidence: 0.8, // Estimated confidence for forced end
            };

            self.speech_segments.push_back(segment);
            self.current_speech.clear();
            self.in_speech = false;
        }

        // Extract all remaining segments
        while let Some(segment) = self.speech_segments.pop_front() {
            completed_segments.push(segment);
        }

        Ok(completed_segments)
    }

    fn process_chunk(&mut self, chunk: &[f32], valid_samples: usize) -> Result<()> {
        // Track accumulated speech buffer size to detect memory issues
        let current_speech_size = self.current_speech.len();
        if current_speech_size > 1_000_000 {
            // More than ~62 seconds of accumulated speech at 16kHz
            warn!("VAD: Accumulated speech buffer is large: {} samples ({:.1}s) - possible memory issue",
                  current_speech_size, current_speech_size as f64 / 16000.0);
        }

        let probability = self.session.probability(chunk)?;
        let valid = &chunk[..valid_samples];

        if !self.in_speech && probability >= self.positive_speech_threshold {
            self.in_speech = true;
            self.last_logged_state = true;
            self.silence_samples = 0;
            self.speech_start_sample = self.processed_samples.saturating_sub(self.pre_speech.len());
            self.current_speech = self.pre_speech.iter().copied().collect();
            debug!(
                "VAD: Speech started at {:.0}ms (probability={probability:.3})",
                self.speech_start_sample as f64 * 1000.0 / VAD_SAMPLE_RATE as f64
            );
        }

        if self.in_speech {
            self.current_speech.extend_from_slice(valid);
            if probability < self.negative_speech_threshold {
                self.silence_samples += valid.len();
            } else {
                self.silence_samples = 0;
            }

            if self.silence_samples >= self.redemption_samples {
                let keep_silence = self.post_speech_pad_samples.min(self.silence_samples);
                let trim = self.silence_samples - keep_silence;
                let end_len = self.current_speech.len().saturating_sub(trim);
                let samples = self.current_speech[..end_len].to_vec();
                if samples.len() >= self.min_speech_samples {
                    let start_timestamp_ms =
                        self.speech_start_sample as f64 * 1000.0 / VAD_SAMPLE_RATE as f64;
                    let end_timestamp_ms =
                        start_timestamp_ms + samples.len() as f64 * 1000.0 / VAD_SAMPLE_RATE as f64;
                    info!(
                        "VAD: Completed speech segment: {:.1}ms duration, {} samples",
                        end_timestamp_ms - start_timestamp_ms,
                        samples.len()
                    );
                    self.speech_segments.push_back(SpeechSegment {
                        samples,
                        start_timestamp_ms,
                        end_timestamp_ms,
                        confidence: probability,
                    });
                }
                self.current_speech.clear();
                self.in_speech = false;
                self.last_logged_state = false;
                self.silence_samples = 0;
            }
        }

        self.pre_speech.extend(valid.iter().copied());
        while self.pre_speech.len() > self.pre_speech_pad_samples {
            self.pre_speech.pop_front();
        }
        self.processed_samples += valid.len();
        Ok(())
    }
}

/// Legacy function for backward compatibility - now uses the optimized approach
pub fn extract_speech_16k(samples_mono_16k: &[f32]) -> Result<Vec<f32>> {
    let mut processor = ContinuousVadProcessor::new(16000, 400)?;

    // Process all audio
    let mut all_segments = processor.process_audio(samples_mono_16k)?;
    let final_segments = processor.flush()?;
    all_segments.extend(final_segments);

    // Concatenate all speech segments
    let mut result = Vec::new();
    let num_segments = all_segments.len();
    for segment in &all_segments {
        result.extend_from_slice(&segment.samples);
    }

    // Apply balanced energy filtering for very short segments
    if result.len() < 1600 {
        // Less than 100ms at 16kHz
        let input_energy: f32 =
            samples_mono_16k.iter().map(|&x| x * x).sum::<f32>() / samples_mono_16k.len() as f32;
        let rms = input_energy.sqrt();
        let peak = samples_mono_16k
            .iter()
            .map(|&x| x.abs())
            .fold(0.0f32, f32::max);

        // BALANCED FIX: Lowered thresholds to preserve quiet speech while still filtering silence
        // Previous aggressive values (0.08/0.15) were discarding valid quiet speech
        // New values (0.03/0.08) are more balanced - catch quiet speech, reject pure silence
        if rms < 0.2 || peak < 0.20 {
            info!("-----VAD detected silence/noise (RMS: {:.6}, Peak: {:.6}), skipping to prevent hallucinations-----", rms, peak);
            return Ok(Vec::new());
        } else {
            info!(
                "VAD detected speech with sufficient energy (RMS: {:.6}, Peak: {:.6})",
                rms, peak
            );
            return Ok(samples_mono_16k.to_vec());
        }
    }

    debug!(
        "VAD: Processed {} samples, extracted {} speech samples from {} segments",
        samples_mono_16k.len(),
        result.len(),
        num_segments
    );

    Ok(result)
}

/// Simple convenience function to get speech chunks from audio
/// Uses the optimized ContinuousVadProcessor with configurable redemption time
pub fn get_speech_chunks(
    samples_mono_16k: &[f32],
    redemption_time_ms: u32,
) -> Result<Vec<SpeechSegment>> {
    get_speech_chunks_with_progress(samples_mono_16k, redemption_time_ms, |_, _| true)
}

/// Get speech chunks with progress callback and cancellation support
/// The callback receives (progress_percent, segments_found) and returns false to cancel
pub fn get_speech_chunks_with_progress<F>(
    samples_mono_16k: &[f32],
    redemption_time_ms: u32,
    mut progress_callback: F,
) -> Result<Vec<SpeechSegment>>
where
    F: FnMut(u32, usize) -> bool,
{
    let mut processor = ContinuousVadProcessor::new(16000, redemption_time_ms)?;

    let total_samples = samples_mono_16k.len();

    // For large files (>1 minute at 16kHz = 960,000 samples), process in chunks with progress logging
    const LARGE_FILE_THRESHOLD: usize = 960_000;
    const CHUNK_SIZE: usize = 160_000; // 10 seconds at 16kHz

    let mut all_segments = Vec::new();

    if total_samples > LARGE_FILE_THRESHOLD {
        info!(
            "VAD: Processing large file ({} samples = {:.1}s), will log progress...",
            total_samples,
            total_samples as f64 / 16000.0
        );

        let mut processed = 0;
        let mut last_progress = 0u32;
        let mut chunk_count = 0;
        let total_chunks = (total_samples + CHUNK_SIZE - 1) / CHUNK_SIZE;

        for chunk in samples_mono_16k.chunks(CHUNK_SIZE) {
            chunk_count += 1;

            let start_time = std::time::Instant::now();
            let segments = processor.process_audio(chunk)?;
            let elapsed = start_time.elapsed();

            // Debug log for chunk processing details
            debug!(
                "VAD: Chunk {}/{} processed in {:?}, found {} segments",
                chunk_count,
                total_chunks,
                elapsed,
                segments.len()
            );

            // Warn if chunk processing took too long (>1 second)
            if elapsed.as_secs() > 1 {
                warn!(
                    "VAD: Chunk {} took {:?} - possible performance issue",
                    chunk_count, elapsed
                );
            }

            all_segments.extend(segments);

            processed += chunk.len();
            let progress = ((processed * 100) / total_samples) as u32;

            // Call progress callback every 5%
            if progress >= last_progress + 5 {
                debug!(
                    "VAD: Progress {}% ({} segments found so far)",
                    progress,
                    all_segments.len()
                );

                // Check for cancellation
                if !progress_callback(progress, all_segments.len()) {
                    info!("VAD: Cancelled by callback at {}%", progress);
                    return Err(anyhow!("VAD processing cancelled"));
                }

                last_progress = progress;
            }
        }

        let final_segments = processor.flush()?;
        all_segments.extend(final_segments);

        info!(
            "VAD: Complete! Found {} speech segments",
            all_segments.len()
        );
    } else {
        // Small file - process all at once
        all_segments = processor.process_audio(samples_mono_16k)?;
        let final_segments = processor.flush()?;
        all_segments.extend(final_segments);
    }

    Ok(all_segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests that build a real [`VadProcessor`] are `#[ignore]`d: they need a
    /// live ONNX Runtime, and on Windows `ort` is built with `load-dynamic`, so
    /// the only thing that ever points it at the bundled `onnxruntime.dll` is
    /// `ort::init_from(...)` in the app's `setup()`. A bare `cargo test`
    /// process never runs that, so session creation times out rather than
    /// failing on anything this module is responsible for.
    ///
    /// Run them from a shell that can supply the runtime:
    /// `cargo test -p meetily audio::vad -- --ignored`.
    const _REQUIRES_ONNX_RUNTIME: () = ();

    /// Generate synthetic speech-like audio with alternating speech/silence
    fn generate_test_audio_with_speech(duration_seconds: f32, sample_rate: u32) -> Vec<f32> {
        let total_samples = (duration_seconds * sample_rate as f32) as usize;
        let mut samples = vec![0.0f32; total_samples];

        // Create speech-like patterns: bursts of sine waves with varying amplitude
        // Speech every 10 seconds for 5 seconds
        let speech_interval = 10.0; // seconds between speech starts
        let speech_duration = 5.0; // seconds of speech

        for i in 0..total_samples {
            let time = i as f32 / sample_rate as f32;
            let cycle_time = time % speech_interval;

            // Speech occurs in the first `speech_duration` seconds of each cycle
            if cycle_time < speech_duration {
                // Generate speech-like signal: multiple frequencies with amplitude modulation
                let freq1 = 200.0 + (time * 50.0).sin() * 100.0; // Varying fundamental
                let freq2 = freq1 * 2.0; // Harmonic
                let freq3 = freq1 * 3.0; // Another harmonic

                let amplitude = 0.3 + 0.1 * (time * 5.0).sin(); // Amplitude modulation
                samples[i] = amplitude
                    * (0.5 * (2.0 * std::f32::consts::PI * freq1 * time).sin()
                        + 0.3 * (2.0 * std::f32::consts::PI * freq2 * time).sin()
                        + 0.2 * (2.0 * std::f32::consts::PI * freq3 * time).sin());
            }
            // else: silence (already 0.0)
        }

        samples
    }

    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_vad_chunked_vs_single_processing() {
        // Generate 60 seconds of audio with speech patterns at 16kHz
        let audio = generate_test_audio_with_speech(60.0, 16000);
        println!(
            "Generated {} samples ({:.1}s)",
            audio.len(),
            audio.len() as f32 / 16000.0
        );

        // Process all at once (like small files)
        let segments_single = get_speech_chunks(&audio, 2000).expect("Single processing failed");
        println!("Single processing found {} segments", segments_single.len());

        // Process in chunks (like large files)
        let segments_chunked =
            get_speech_chunks_with_progress(&audio, 2000, |progress, segments| {
                println!("Chunked progress: {}%, {} segments", progress, segments);
                true // Don't cancel
            })
            .expect("Chunked processing failed");
        println!(
            "Chunked processing found {} segments",
            segments_chunked.len()
        );

        // Both should find the same number of segments (approximately)
        // Allow some variance due to chunk boundary effects
        let diff = (segments_single.len() as i32 - segments_chunked.len() as i32).abs();
        assert!(
            diff <= 1,
            "Chunked and single processing found different segment counts: {} vs {} (diff: {})",
            segments_single.len(),
            segments_chunked.len(),
            diff
        );
    }

    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_vad_large_file_progress() {
        // Generate 120 seconds (2 minutes) of audio - triggers large file threshold
        let audio = generate_test_audio_with_speech(120.0, 16000);
        let total_samples = audio.len();
        println!(
            "Generated {} samples ({:.1}s)",
            total_samples,
            total_samples as f32 / 16000.0
        );

        // This should trigger the large file path (>960,000 samples)
        assert!(
            total_samples > 960_000,
            "Audio should be large enough to trigger chunked processing"
        );

        let mut progress_updates = Vec::new();
        let segments = get_speech_chunks_with_progress(&audio, 2000, |progress, segments| {
            progress_updates.push((progress, segments));
            true // Don't cancel
        })
        .expect("Processing failed");

        println!(
            "Found {} segments with {} progress updates",
            segments.len(),
            progress_updates.len()
        );

        // The synthetic signal is not real speech, so Silero may merge it into
        // one long segment. This test is specifically for the large-file path:
        // it must still emit speech and report monotonic progress through 100%.
        assert!(!segments.is_empty(), "Expected at least one speech segment");
        assert!(
            segments.iter().all(|segment| !segment.samples.is_empty()
                && segment.end_timestamp_ms > segment.start_timestamp_ms),
            "Expected all speech segments to contain audio with positive duration"
        );

        // Should have received progress updates
        assert!(
            !progress_updates.is_empty(),
            "Expected progress updates for large file"
        );
        assert_eq!(
            progress_updates.last().map(|(progress, _)| *progress),
            Some(100),
            "Expected progress to reach 100%"
        );
        assert!(
            progress_updates
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "Expected progress updates to increase monotonically: {:?}",
            progress_updates
        );
    }

    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_vad_cancellation() {
        let audio = generate_test_audio_with_speech(120.0, 16000);

        // Cancel at 50%
        let result = get_speech_chunks_with_progress(&audio, 2000, |progress, _| {
            progress < 50 // Cancel when reaching 50%
        });

        // Should return error due to cancellation
        assert!(result.is_err(), "Expected cancellation error");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cancelled"),
            "Error should mention cancellation: {}",
            err_msg
        );
    }

    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_vad_continuous_processor_state_across_chunks() {
        // Test that VAD state is correctly maintained across chunk boundaries
        let mut processor =
            ContinuousVadProcessor::new(16000, 2000).expect("Failed to create processor");

        // Generate audio with a speech segment that spans a chunk boundary
        let chunk_size = 160_000; // 10 seconds
        let audio = generate_test_audio_with_speech(30.0, 16000); // 30 seconds

        // Process in 10-second chunks
        let mut all_segments = Vec::new();
        for (i, chunk) in audio.chunks(chunk_size).enumerate() {
            let segments = processor.process_audio(chunk).expect("Processing failed");
            println!(
                "Chunk {}: processed {} samples, found {} segments",
                i,
                chunk.len(),
                segments.len()
            );
            all_segments.extend(segments);
        }

        // Flush remaining
        let final_segments = processor.flush().expect("Flush failed");
        all_segments.extend(final_segments);

        println!("Total segments found: {}", all_segments.len());

        // Should find speech segments
        assert!(
            all_segments.len() >= 1,
            "Expected at least 1 speech segment"
        );
    }

    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_vad_400ms_vs_2000ms_segmentation() {
        // Demonstrates why 2000ms redemption is needed for batch processing:
        // 400ms creates excessive fragmentation, 2000ms bridges natural pauses.
        //
        // Audio pattern: 60s with 5s speech / 5s silence cycles
        // Natural pauses within speech (sentence gaps) are 500ms-1.5s
        let audio = generate_test_audio_with_speech(60.0, 16000);

        let segments_400 = get_speech_chunks(&audio, 400).expect("400ms processing failed");
        let segments_2000 = get_speech_chunks(&audio, 2000).expect("2000ms processing failed");

        println!(
            "400ms redemption: {} segments, 2000ms redemption: {} segments",
            segments_400.len(),
            segments_2000.len()
        );

        // 2000ms should produce fewer or equal segments (bridges more pauses)
        assert!(
            segments_2000.len() <= segments_400.len(),
            "2000ms redemption ({} segments) should not produce more segments than 400ms ({} segments)",
            segments_2000.len(),
            segments_400.len()
        );

        // Verify segments have reasonable durations with 2000ms
        for (i, seg) in segments_2000.iter().enumerate() {
            let duration_ms = seg.end_timestamp_ms - seg.start_timestamp_ms;
            println!("2000ms segment {}: {:.0}ms duration", i, duration_ms);
            // Each segment should be at least 250ms (min_speech_time)
            assert!(
                duration_ms >= 200.0,
                "Segment {} too short: {:.0}ms",
                i,
                duration_ms
            );
        }
    }
    /// Leading silence, then speech that runs to the end of the buffer.
    ///
    /// This is the shape that matters for the flush path: an utterance that begins
    /// late in a long session and is still in progress when recording stops.
    fn generate_late_speech_audio(
        silence_seconds: f32,
        speech_seconds: f32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let silence_samples = (silence_seconds * sample_rate as f32) as usize;
        let speech = generate_test_audio_with_speech(speech_seconds, sample_rate);

        let mut samples = vec![0.0f32; silence_samples];
        samples.extend_from_slice(&speech);
        samples
    }

    /// `speech_start_sample` records where the current utterance began, so it can
    /// never point past the number of samples the VAD has actually seen.
    ///
    /// It used to, because it was computed as `processed_samples + timestamp_ms` where
    /// silero's `timestamp_ms` is ALREADY session-absolute
    /// (`processed_duration() - pre_speech_pad`), which doubled the position. The only
    /// reader is the force-end branch in `flush()`, so in production the corruption
    /// escaped as one phantom segment per recording, timestamped past the end of the
    /// audio. The error grows with how late the utterance starts, which is why it took
    /// a long recording to surface.
    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_speech_start_sample_never_exceeds_processed_samples() {
        // 20s of silence, then 3s of speech still running when the buffer ends.
        let audio = generate_late_speech_audio(20.0, 3.0, 16000);

        let mut processor =
            ContinuousVadProcessor::new(16000, 2000).expect("Failed to create processor");
        processor
            .process_audio(&audio)
            .expect("process_audio failed");

        assert!(
            processor.in_speech,
            "expected to still be mid-speech at the end of the buffer; the invariant \
             below would not be exercised otherwise"
        );

        assert!(
            processor.speech_start_sample <= processor.processed_samples,
            "speech_start_sample ({}) is past processed_samples ({}) - \
             session-absolute timestamp double-count regression. \
             In seconds: start={:.2}s vs processed={:.2}s",
            processor.speech_start_sample,
            processor.processed_samples,
            processor.speech_start_sample as f64 / VAD_SAMPLE_RATE as f64,
            processor.processed_samples as f64 / VAD_SAMPLE_RATE as f64,
        );
    }

    /// A forced segment's timestamps and samples must describe the same real audio interval.
    #[test]
    #[ignore = "needs a live ONNX Runtime; see _REQUIRES_ONNX_RUNTIME"]
    fn test_flush_segment_timestamps_stay_within_audio_duration() {
        let audio = generate_late_speech_audio(20.0, 3.0, 16000);
        let audio_duration_ms = (audio.len() as f64 / VAD_SAMPLE_RATE as f64) * 1000.0;

        assert_eq!(audio.len(), 368_000);
        assert_eq!(
            audio.len() % SILERO_FRAME_SIZE,
            384,
            "fixture must require 128 samples of terminal VAD padding"
        );

        let mut processor =
            ContinuousVadProcessor::new(16000, 2000).expect("Failed to create processor");

        let segments = processor
            .process_audio(&audio)
            .expect("process_audio failed");
        assert!(
            segments.is_empty(),
            "process_audio completed a segment, so flush() would not exercise force-end"
        );
        assert!(
            processor.in_speech,
            "expected to still be mid-speech before flush()"
        );

        let flushed = processor.flush().expect("flush failed");
        assert_eq!(flushed.len(), 1, "force-end must emit exactly one segment");

        let segment = &flushed[0];
        let start_sample =
            ((segment.start_timestamp_ms / 1000.0) * VAD_SAMPLE_RATE as f64).round() as usize;

        assert!(
            segment.start_timestamp_ms <= audio_duration_ms,
            "segment starts at {:.0}ms, beyond the {:.0}ms of audio supplied",
            segment.start_timestamp_ms,
            audio_duration_ms
        );
        assert_eq!(
            segment.end_timestamp_ms, 23_000.0,
            "forced segment must end at the real audio endpoint"
        );
        assert!(
            segment.end_timestamp_ms <= audio_duration_ms,
            "segment ends at {:.0}ms, beyond the {:.0}ms of audio supplied",
            segment.end_timestamp_ms,
            audio_duration_ms
        );
        assert!(
            segment.end_timestamp_ms >= segment.start_timestamp_ms,
            "segment ends before it starts: {:.0}ms -> {:.0}ms",
            segment.start_timestamp_ms,
            segment.end_timestamp_ms
        );
        assert_eq!(
            segment.samples.as_slice(),
            &audio[start_sample..],
            "forced payload must contain the exact real audio interval named by its timestamps"
        );
        assert_eq!(segment.samples.len(), audio.len() - start_sample);

        let timestamp_sample_count = (((segment.end_timestamp_ms - segment.start_timestamp_ms)
            / 1000.0)
            * VAD_SAMPLE_RATE as f64)
            .round() as usize;
        assert_eq!(
            timestamp_sample_count,
            segment.samples.len(),
            "timestamp duration and payload length must describe the same sample interval"
        );

        let payload_end_ms = segment.start_timestamp_ms
            + (segment.samples.len() as f64 / VAD_SAMPLE_RATE as f64) * 1000.0;
        assert!(
            (payload_end_ms - segment.end_timestamp_ms).abs() < 0.001,
            "payload ends at {payload_end_ms:.3}ms, timestamp ends at {:.3}ms",
            segment.end_timestamp_ms
        );

        assert!(
            processor.flush().expect("second flush failed").is_empty(),
            "flush() must not emit the same forced segment twice"
        );
    }
}
