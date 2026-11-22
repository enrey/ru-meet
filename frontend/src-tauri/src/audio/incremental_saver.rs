use super::encode::encode_recording_streams;
use super::recording_state::RecordingFrame;
use anyhow::{anyhow, Result};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::ffmpeg::find_ffmpeg_path;

/// Incremental audio saver that writes checkpoints every 30 seconds
/// to minimize memory usage and enable crash recovery
pub struct IncrementalAudioSaver {
    /// Mix, microphone and system samples of the current checkpoint, kept
    /// the same length so the file streams stay aligned (ADR 0005).
    mix: Vec<f32>,
    microphone: Vec<f32>,
    system: Vec<f32>,
    checkpoint_interval_samples: usize, // 30s at 48kHz = 1,440,000 samples
    checkpoint_count: u32,
    checkpoints_dir: PathBuf,
    meeting_folder: PathBuf,
    sample_rate: u32,
}

impl IncrementalAudioSaver {
    /// Create a new incremental saver
    ///
    /// # Arguments
    /// * `meeting_folder` - Path to the meeting folder (contains .checkpoints/)
    /// * `sample_rate` - Sample rate of audio (typically 48000)
    pub fn new(meeting_folder: PathBuf, sample_rate: u32) -> Result<Self> {
        let checkpoints_dir = meeting_folder.join(".checkpoints");

        // Verify checkpoints directory exists
        if !checkpoints_dir.exists() {
            return Err(anyhow!(
                "Checkpoints directory does not exist: {}",
                checkpoints_dir.display()
            ));
        }

        Ok(Self {
            mix: Vec::new(),
            microphone: Vec::new(),
            system: Vec::new(),
            checkpoint_interval_samples: sample_rate as usize * 30, // 30 seconds
            checkpoint_count: 0,
            checkpoints_dir,
            meeting_folder,
            sample_rate,
        })
    }

    /// Add one mixing window to the buffer.
    /// Automatically saves a checkpoint when buffer reaches 30 seconds
    pub fn add_frame(&mut self, frame: RecordingFrame) -> Result<()> {
        // The pipeline cuts all three from one window; pad defensively so a
        // mismatch can never shift one stream against the others.
        let len = frame
            .mix
            .len()
            .max(frame.microphone.len())
            .max(frame.system.len());
        for (buffer, samples) in [
            (&mut self.mix, frame.mix),
            (&mut self.microphone, frame.microphone),
            (&mut self.system, frame.system),
        ] {
            let end = buffer.len() + len;
            buffer.extend(samples);
            buffer.resize(end, 0.0);
        }

        // Save checkpoint when buffer reaches threshold (30 seconds)
        if self.mix.len() >= self.checkpoint_interval_samples {
            self.save_checkpoint()?;
        }

        Ok(())
    }

    /// Save the current buffer as a checkpoint file and clear it
    fn save_checkpoint(&mut self) -> Result<()> {
        let audio_data = std::mem::take(&mut self.mix);
        let microphone = std::mem::take(&mut self.microphone);
        let system = std::mem::take(&mut self.system);

        if audio_data.is_empty() {
            warn!("Attempted to save empty checkpoint, skipping");
            return Ok(());
        }

        // Generate checkpoint filename
        let checkpoint_path = self
            .checkpoints_dir
            .join(format!("audio_chunk_{:03}.mp4", self.checkpoint_count));

        // Encode and save checkpoint
        encode_recording_streams(
            &audio_data,
            &microphone,
            &system,
            self.sample_rate,
            &checkpoint_path,
        )?;

        let duration_seconds = audio_data.len() as f32 / self.sample_rate as f32;
        self.checkpoint_count += 1;

        info!(
            "Saved checkpoint {}: {:.2}s of audio ({} samples)",
            self.checkpoint_count,
            duration_seconds,
            audio_data.len()
        );

        Ok(())
    }

    /// Finalize the recording: save final checkpoint, merge all checkpoints, cleanup
    ///
    /// Returns the path to the final merged audio.mp4 file
    pub async fn finalize(&mut self) -> Result<PathBuf> {
        info!("Finalizing incremental recording...");

        // Save final buffer if not empty
        if !self.mix.is_empty() {
            info!(
                "Saving final checkpoint with remaining {} samples",
                self.mix.len()
            );
            self.save_checkpoint()?;
        }

        if self.checkpoint_count == 0 {
            return Err(anyhow!(
                "No audio checkpoints to merge - recording may have failed"
            ));
        }

        // Merge all checkpoints using FFmpeg concat
        let final_audio_path = self.meeting_folder.join("audio.mp4");
        self.merge_checkpoints(&final_audio_path).await?;

        // Clean up checkpoints directory
        info!("Cleaning up {} checkpoint files", self.checkpoint_count);
        if let Err(e) = std::fs::remove_dir_all(&self.checkpoints_dir) {
            warn!("Failed to clean up checkpoints directory: {}", e);
            // Non-fatal - user can manually delete
        }

        info!("Finalized recording: {}", final_audio_path.display());

        Ok(final_audio_path)
    }

    /// Merge all checkpoint files into final audio.mp4 using FFmpeg concat
    /// Uses concat demuxer for fast merging without re-encoding
    async fn merge_checkpoints(&self, output: &PathBuf) -> Result<()> {
        info!(
            "Merging {} checkpoints into final audio file...",
            self.checkpoint_count
        );

        // Create concat list file for FFmpeg
        let list_file = self.checkpoints_dir.join("concat_list.txt");
        let mut list_content = String::new();

        for i in 0..self.checkpoint_count {
            let checkpoint_path = self
                .checkpoints_dir
                .join(format!("audio_chunk_{:03}.mp4", i));

            // Verify checkpoint exists
            if !checkpoint_path.exists() {
                return Err(anyhow!(
                    "Checkpoint file missing: {}",
                    checkpoint_path.display()
                ));
            }

            // Use absolute path for FFmpeg (required for safe mode)
            let abs_path = checkpoint_path.canonicalize()?;
            list_content.push_str(&format!("file '{}'\n", abs_path.display()));
        }

        std::fs::write(&list_file, list_content)?;

        let ffmpeg_path = find_ffmpeg_path().ok_or_else(|| {
            anyhow!("FFmpeg not found. Please install FFmpeg to finalize recordings.")
        })?;
        info!("Using FFmpeg at: {:?}", ffmpeg_path);

        // Run FFmpeg concat command
        // Using concat demuxer with copy codec for fast merging (no re-encoding)

        let mut command = std::process::Command::new(ffmpeg_path);

        command.args(&[
            "-f",
            "concat", // Use concat demuxer
            "-safe",
            "0", // Allow absolute paths
            "-i",
            list_file.to_str().unwrap(),
            // Keep every stream: ffmpeg otherwise picks one audio stream and
            // silently drops the microphone and system tracks (ADR 0005).
            "-map",
            "0",
            "-c",
            "copy", // Copy codec - no re-encoding!
            // Index up front: WebKit on macOS cannot play an MP4 whose
            // index trails the audio through the asset protocol.
            "-movflags",
            "+faststart",
            "-y", // Overwrite output file
            output.to_str().unwrap(),
        ]);

        // Hide console window on Windows to prevent CMD popup during finalization
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let ffmpeg_output = command.output()?;

        if !ffmpeg_output.status.success() {
            let stderr = String::from_utf8_lossy(&ffmpeg_output.stderr);
            error!("FFmpeg merge failed: {}", stderr);
            return Err(anyhow!("FFmpeg concat failed: {}", stderr));
        }

        // Verify output file was created
        if !output.exists() {
            return Err(anyhow!(
                "Merged audio file was not created: {}",
                output.display()
            ));
        }

        info!(
            "Successfully merged {} checkpoints → {}",
            self.checkpoint_count,
            output.display()
        );

        Ok(())
    }

    /// Get the meeting folder path
    pub fn get_meeting_folder(&self) -> &PathBuf {
        &self.meeting_folder
    }

    /// Get current checkpoint count
    pub fn get_checkpoint_count(&self) -> u32 {
        self.checkpoint_count
    }
}

/// Audio recovery status for transcript recovery feature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioRecoveryStatus {
    pub status: String, // "success" | "partial" | "failed" | "none"
    pub chunk_count: u32,
    pub estimated_duration_seconds: f64,
    pub audio_file_path: Option<String>,
    pub message: String,
}

/// Recover audio from checkpoint files
/// This is called by the transcript recovery system to merge audio chunks after a crash
#[tauri::command]
pub async fn recover_audio_from_checkpoints(
    meeting_folder: String,
    _sample_rate: u32,
) -> Result<AudioRecoveryStatus, String> {
    info!("Starting audio recovery for folder: {}", meeting_folder);

    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_dir = folder_path.join(".checkpoints");

    // Check if checkpoints directory exists
    if !checkpoints_dir.exists() {
        info!(
            "No checkpoints directory found at: {}",
            checkpoints_dir.display()
        );
        return Ok(AudioRecoveryStatus {
            status: "none".to_string(),
            chunk_count: 0,
            estimated_duration_seconds: 0.0,
            audio_file_path: None,
            message: "No audio checkpoints found".to_string(),
        });
    }

    // Scan for checkpoint files
    let mut checkpoint_files: Vec<_> = std::fs::read_dir(&checkpoints_dir)
        .map_err(|e| format!("Failed to read checkpoints directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("mp4"))
        .collect();

    if checkpoint_files.is_empty() {
        info!(
            "No checkpoint files found in: {}",
            checkpoints_dir.display()
        );
        return Ok(AudioRecoveryStatus {
            status: "none".to_string(),
            chunk_count: 0,
            estimated_duration_seconds: 0.0,
            audio_file_path: None,
            message: "No audio checkpoint files found".to_string(),
        });
    }

    // Sort by filename (audio_chunk_000.mp4, audio_chunk_001.mp4, etc.)
    checkpoint_files.sort_by_key(|entry| entry.path());

    let chunk_count = checkpoint_files.len() as u32;
    let estimated_duration = (chunk_count as f64) * 30.0; // 30 seconds per chunk

    info!(
        "Found {} checkpoint files, estimated duration: {:.2}s",
        chunk_count, estimated_duration
    );

    // Create FFmpeg concat file
    let concat_file_path = checkpoints_dir.join("concat_list.txt");
    let mut concat_content = String::new();

    for entry in &checkpoint_files {
        let path = entry
            .path()
            .canonicalize()
            .map_err(|e| format!("Failed to canonicalize path: {}", e))?;
        concat_content.push_str(&format!("file '{}'\n", path.display()));
    }

    std::fs::write(&concat_file_path, concat_content)
        .map_err(|e| format!("Failed to write concat file: {}", e))?;

    // Run FFmpeg to merge chunks
    let output_path = folder_path.join("audio.mp4");
    let output_path_str = output_path
        .to_str()
        .ok_or("Invalid output path")?
        .to_string();

    let ffmpeg_path = find_ffmpeg_path()
        .ok_or_else(|| "FFmpeg not found. Please install FFmpeg to recover audio.".to_string())?;
    info!("Using FFmpeg at: {:?}", ffmpeg_path);

    let mut command = std::process::Command::new(ffmpeg_path);

    command.args(&[
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
        concat_file_path.to_str().unwrap(),
        // Keep the microphone and system streams (ADR 0005).
        "-map",
        "0",
        "-c",
        "copy",
        "-movflags",
        "+faststart",
        "-y", // Overwrite if exists
        &output_path_str,
    ]);

    // Hide console window on Windows
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let ffmpeg_result = command.output();

    match ffmpeg_result {
        Ok(output) if output.status.success() => {
            // Clean up concat file
            let _ = std::fs::remove_file(concat_file_path);

            info!("Successfully recovered audio: {}", output_path_str);

            Ok(AudioRecoveryStatus {
                status: "success".to_string(),
                chunk_count,
                estimated_duration_seconds: estimated_duration,
                audio_file_path: Some(output_path_str),
                message: format!("Successfully recovered {} audio chunks", chunk_count),
            })
        }
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr);
            error!("FFmpeg recovery failed: {}", error);
            Ok(AudioRecoveryStatus {
                status: "failed".to_string(),
                chunk_count,
                estimated_duration_seconds: estimated_duration,
                audio_file_path: None,
                message: format!("FFmpeg failed: {}", error),
            })
        }
        Err(e) => {
            error!("Failed to run FFmpeg: {}", e);
            Ok(AudioRecoveryStatus {
                status: "failed".to_string(),
                chunk_count,
                estimated_duration_seconds: estimated_duration,
                audio_file_path: None,
                message: format!("Failed to run FFmpeg: {}", e),
            })
        }
    }
}

/// Clean up checkpoint files after successful recording or recovery
/// This command is called by the frontend after successful save to clean up checkpoint files
#[tauri::command]
pub async fn cleanup_checkpoints(meeting_folder: String) -> Result<(), String> {
    info!("Cleaning up checkpoints for folder: {}", meeting_folder);

    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_dir = folder_path.join(".checkpoints");

    if checkpoints_dir.exists() {
        std::fs::remove_dir_all(&checkpoints_dir)
            .map_err(|e| format!("Failed to remove checkpoints directory: {}", e))?;
        info!("Successfully cleaned up checkpoints directory");
    } else {
        info!("No checkpoints directory to clean up");
    }

    Ok(())
}

/// Check if a meeting folder has audio checkpoint files
/// Returns true if .checkpoints/ directory exists and contains .mp4 files
#[tauri::command]
pub async fn has_audio_checkpoints(meeting_folder: String) -> Result<bool, String> {
    let folder_path = PathBuf::from(&meeting_folder);
    let checkpoints_dir = folder_path.join(".checkpoints");

    // Check if checkpoints directory exists
    if !checkpoints_dir.exists() {
        return Ok(false);
    }

    // Scan for .mp4 checkpoint files
    let has_mp4_files = std::fs::read_dir(&checkpoints_dir)
        .map_err(|e| format!("Failed to read checkpoints directory: {}", e))?
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("mp4"));

    Ok(has_mp4_files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_checkpoint_creation() {
        // Create temp meeting folder
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Test_Meeting");
        std::fs::create_dir_all(&meeting_folder).unwrap();
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();

        let mut saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000).unwrap();

        // Add 60 seconds worth of audio (should create 2 checkpoints)
        for _ in 0..120 {
            // 120 frames of 0.5s each at 48kHz
            saver
                .add_frame(RecordingFrame {
                    mix: vec![0.5f32; 24000],
                    microphone: vec![0.25f32; 24000],
                    system: vec![0.25f32; 24000],
                })
                .unwrap();
        }

        // Verify 2 checkpoints created
        assert_eq!(saver.checkpoint_count, 2);

        // Finalize and verify merge
        let final_path = saver.finalize().await.unwrap();
        assert!(final_path.exists());

        // Verify checkpoints directory deleted
        assert!(!meeting_folder.join(".checkpoints").exists());
    }

    /// The file keeps three aligned streams through checkpoint merging and
    /// edge trimming, and each decodes on its own (ADR 0005).
    #[tokio::test]
    async fn recording_keeps_aligned_source_streams() {
        use crate::audio::decoder::{decode_recording_stream, RecordingStream};

        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Sources");
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();
        let mut saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000).unwrap();

        // 70 s: a microphone tone, a quieter system tone, and one click in
        // both sources at 31 s, just past the first checkpoint joint.
        let rate = 48000usize;
        let click = 31 * rate;
        let window = rate * 6 / 10;
        for start in (0..70 * rate).step_by(window) {
            let mut frame = RecordingFrame {
                mix: Vec::with_capacity(window),
                microphone: Vec::with_capacity(window),
                system: Vec::with_capacity(window),
            };
            for i in start..start + window {
                let t = i as f32 / rate as f32;
                let near_click = i.abs_diff(click) < 48;
                let microphone = if near_click {
                    0.9
                } else {
                    0.3 * (t * 440.0 * std::f32::consts::TAU).sin()
                };
                let system = if near_click {
                    0.9
                } else {
                    0.05 * (t * 220.0 * std::f32::consts::TAU).sin()
                };
                frame.microphone.push(microphone);
                frame.system.push(system);
                frame.mix.push((microphone + system) / 2.0);
            }
            saver.add_frame(frame).unwrap();
        }
        let path = saver.finalize().await.unwrap();
        super::super::edge_trim::trim_audio_file(&path, 1.0, 66.0).unwrap();

        let decode = |stream| {
            decode_recording_stream(&path, stream)
                .unwrap()
                .expect("stream present")
                .samples
        };
        let (mix, microphone, system) = (
            decode(RecordingStream::Mix),
            decode(RecordingStream::Microphone),
            decode(RecordingStream::System),
        );
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        assert!(
            (rms(&microphone) - 0.21).abs() < 0.03,
            "microphone rms {}",
            rms(&microphone)
        );
        assert!(
            (rms(&system) - 0.035).abs() < 0.01,
            "system rms {}",
            rms(&system)
        );
        assert!(rms(&mix) > rms(&system) && rms(&mix) < rms(&microphone));
        for stream in [&mix, &system] {
            assert!(
                stream.len().abs_diff(microphone.len()) <= 2048,
                "streams must cover the same time"
            );
        }

        // The click sits at 30 s of the trimmed file in every stream.
        let peak = |s: &[f32]| {
            s.iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0 as i64
        };
        // Absolute position is only as exact as AAC allows: the decoder keeps
        // one frame of encoder priming (~21 ms) and a stream-copy trim starts
        // on a packet boundary, so allow two frames plus slack.
        let expected = 30 * rate as i64;
        for (name, stream) in [
            ("microphone", &microphone),
            ("system", &system),
            ("mix", &mix),
        ] {
            let offset = peak(stream) - expected;
            assert!(offset.abs() < 2400, "{name} click off by {offset} samples");
        }
        // Relative to each other the streams must not move; the click is 96
        // samples wide and lossy coding may shift its peak within it.
        for (name, stream) in [("system", &system), ("mix", &mix)] {
            let offset = peak(stream) - peak(&microphone);
            assert!(
                offset.abs() < 128,
                "{name} off the microphone by {offset} samples"
            );
        }
    }

    #[test]
    fn single_stream_files_fall_back_to_the_mix() {
        use crate::audio::decoder::{decode_recording_stream, RecordingStream};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/jfk.wav");
        assert!(decode_recording_stream(&path, RecordingStream::Microphone)
            .unwrap()
            .is_none());
        assert!(decode_recording_stream(&path, RecordingStream::System)
            .unwrap()
            .is_none());
        assert!(decode_recording_stream(&path, RecordingStream::Mix)
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_empty_recording() {
        let temp_dir = tempdir().unwrap();
        let meeting_folder = temp_dir.path().join("Empty_Test");
        std::fs::create_dir_all(&meeting_folder).unwrap();
        std::fs::create_dir_all(meeting_folder.join(".checkpoints")).unwrap();

        let mut saver = IncrementalAudioSaver::new(meeting_folder.clone(), 48000).unwrap();

        // Try to finalize without adding any chunks
        let result = saver.finalize().await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No audio checkpoints"));
    }
}
