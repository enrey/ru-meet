use super::ffmpeg::find_ffmpeg_path; // Correct path to encode module
use super::AudioDevice;
use std::io::Write;
use std::sync::Arc;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};
use tracing::{debug, error};

pub struct AudioInput {
    pub data: Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub device: Arc<AudioDevice>,
}

/// Bitrate of the mix stream; also used for single-stream files.
const MIX_BITRATE: &str = "192k";
/// Bitrate of each source stream in a recording. Speech recognition runs at
/// 16 kHz, so this keeps everything it needs at a third of the mix's size.
const SOURCE_BITRATE: &str = "64k";

pub fn encode_single_audio(
    data: &[u8],
    sample_rate: u32,
    channels: u16,
    output_path: &PathBuf,
) -> anyhow::Result<()> {
    let sample_rate = sample_rate.to_string();
    let channels = channels.to_string();
    let output = output_path.to_string_lossy();
    run_ffmpeg_on_stdin(
        data,
        &[
            "-f",
            "f32le",
            "-ar",
            &sample_rate,
            "-ac",
            &channels,
            "-i",
            "pipe:0",
            "-c:a",
            "aac",
            "-b:a",
            MIX_BITRATE,
            "-profile:a",
            "aac_low", // Use AAC-LC profile for better compatibility
            "-movflags",
            "+faststart", // Optimize for web streaming
            "-f",
            "mp4",
            &output,
        ],
    )
}

/// Encode one recording segment as an MP4 with three mono AAC streams in a
/// fixed order: 0 = mix (default, what players and decoders pick), 1 =
/// microphone, 2 = system audio. The order is the contract (ADR 0005);
/// `handler_name` only labels the streams for people and `ffprobe`.
///
/// All three inputs must have the same length: they are interleaved into one
/// three-channel pipe and split by ffmpeg, which keeps them sample-aligned.
pub fn encode_recording_streams(
    mix: &[f32],
    microphone: &[f32],
    system: &[f32],
    sample_rate: u32,
    output_path: &PathBuf,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        mix.len() == microphone.len() && mix.len() == system.len(),
        "Recording streams differ in length: mix={} microphone={} system={}",
        mix.len(),
        microphone.len(),
        system.len()
    );
    let interleaved: Vec<f32> = mix
        .iter()
        .zip(microphone)
        .zip(system)
        .flat_map(|((&mix, &microphone), &system)| [mix, microphone, system])
        .collect();
    let sample_rate = sample_rate.to_string();
    let output = output_path.to_string_lossy();
    run_ffmpeg_on_stdin(
        bytemuck::cast_slice(&interleaved),
        &[
            "-f",
            "f32le",
            "-ar",
            &sample_rate,
            "-ac",
            "3",
            "-i",
            "pipe:0",
            // Select channels by index, independent of any channel layout.
            "-filter_complex",
            "[0:a]asplit=3[a][b][c];[a]pan=mono|c0=c0[mix];[b]pan=mono|c0=c1[microphone];[c]pan=mono|c0=c2[system]",
            "-map",
            "[mix]",
            "-map",
            "[microphone]",
            "-map",
            "[system]",
            "-c:a",
            "aac",
            "-profile:a",
            "aac_low",
            "-b:a:0",
            MIX_BITRATE,
            "-b:a:1",
            SOURCE_BITRATE,
            "-b:a:2",
            SOURCE_BITRATE,
            "-metadata:s:a:0",
            "handler_name=mix",
            "-metadata:s:a:1",
            "handler_name=microphone",
            "-metadata:s:a:2",
            "handler_name=system",
            "-disposition:a:0",
            "default",
            "-disposition:a:1",
            "0",
            "-disposition:a:2",
            "0",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
            "-y",
            &output,
        ],
    )
}

fn run_ffmpeg_on_stdin(data: &[u8], args: &[&str]) -> anyhow::Result<()> {
    debug!(
        "Starting FFmpeg process for {} bytes of audio data",
        data.len()
    );

    if data.is_empty() {
        return Err(anyhow::anyhow!("No audio data provided for encoding"));
    }

    let ffmpeg_path = find_ffmpeg_path().ok_or_else(|| {
        anyhow::anyhow!("FFmpeg not found. Please install FFmpeg to save recordings.")
    })?;

    debug!("Using FFmpeg at: {:?}", ffmpeg_path);

    let mut command = Command::new(ffmpeg_path);
    command
        .args(["-hide_banner", "-loglevel", "error"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Hide console window on Windows to prevent CMD popup during recording
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    debug!("FFmpeg command: {:?}", command);

    let mut ffmpeg = command.spawn()?;
    debug!("FFmpeg process spawned");
    let mut stdin = ffmpeg
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Failed to open FFmpeg stdin"))?;

    // stderr is piped and read only after exit; at `-loglevel error` ffmpeg
    // writes too little to fill the pipe while we feed stdin.
    let written = stdin.write_all(data);
    drop(stdin);
    let output = ffmpeg.wait_with_output()?;
    let status = output.status;
    let stderr = String::from_utf8_lossy(&output.stderr);

    debug!("FFmpeg process exited with status: {}", status);
    debug!("FFmpeg stderr: {}", stderr);

    if !status.success() {
        error!("FFmpeg process failed with status: {}", status);
        error!("FFmpeg stderr: {}", stderr);
        return Err(anyhow::anyhow!(
            "FFmpeg process failed with status: {}",
            status
        ));
    }
    written?;

    Ok(())
}
