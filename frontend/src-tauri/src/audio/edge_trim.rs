//! Cut the silence before the first and after the last phrase of a finished
//! recording, so playback and diarization do not start with minutes of an
//! empty lobby or end with the room after everyone left.
//!
//! The speech bounds come from the transcript itself: its segments are what
//! the transcription VAD found in the same recording, so no second pass over
//! the audio is needed. The cut happens before the meeting is saved to the
//! database, so only the in-memory segments need shifting.
use super::recording_saver::TranscriptSegment;
use anyhow::{anyhow, Result};
use std::path::Path;

/// Audio kept around the first and last phrase.
const EDGE_MARGIN: f64 = 2.0;
/// Shorter silence is not worth rewriting the file for.
const MIN_TRIM: f64 = 5.0;

/// The part of the recording to keep, in seconds, or `None` to keep it all.
/// `duration` is the recording length when known; an infinite end keeps the
/// tail.
pub(crate) fn speech_window(
    segments: &[TranscriptSegment],
    duration: Option<f64>,
) -> Option<(f64, f64)> {
    let first = segments
        .iter()
        .map(|segment| segment.audio_start_time)
        .min_by(f64::total_cmp)?;
    let last = segments
        .iter()
        .map(|segment| segment.audio_end_time)
        .max_by(f64::total_cmp)?;
    let start = (first - EDGE_MARGIN).max(0.0);
    let end = last + EDGE_MARGIN;
    let trim_head = start >= MIN_TRIM;
    let trim_tail = duration.is_some_and(|duration| duration - end >= MIN_TRIM);
    match (trim_head, trim_tail) {
        (false, false) => None,
        (true, false) => Some((start, f64::INFINITY)),
        (false, true) => Some((0.0, end)),
        (true, true) => Some((start, end)),
    }
}

/// Move segments onto the timeline of the trimmed file.
pub(crate) fn shift_segments(segments: &mut [TranscriptSegment], offset: f64) {
    for segment in segments {
        segment.audio_start_time = (segment.audio_start_time - offset).max(0.0);
        segment.audio_end_time = (segment.audio_end_time - offset).max(0.0);
    }
}

/// Replace `path` with its `[start, end)` part without re-encoding.
pub(crate) fn trim_audio_file(path: &Path, start: f64, end: f64) -> Result<()> {
    let ffmpeg = super::ffmpeg::find_ffmpeg_path().ok_or_else(|| anyhow!("FFmpeg not found"))?;
    let trimmed = path.with_file_name(".audio.trimmed.mp4");
    let mut command = std::process::Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{start:.3}"))
        .arg("-i")
        .arg(path);
    if end.is_finite() {
        command.arg("-t").arg(format!("{:.3}", end - start));
    }
    command
        .args(["-c", "copy", "-movflags", "+faststart", "-y"])
        .arg(&trimmed);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output()?;
    if !output.status.success() || !trimmed.is_file() {
        let _ = std::fs::remove_file(&trimmed);
        return Err(anyhow!(
            "FFmpeg trim failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    std::fs::rename(&trimmed, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: f64, end: f64) -> TranscriptSegment {
        TranscriptSegment {
            id: String::new(),
            text: String::new(),
            audio_start_time: start,
            audio_end_time: end,
            duration: end - start,
            display_time: String::new(),
            confidence: 1.0,
            sequence_id: 0,
            speaker: None,
        }
    }

    #[test]
    fn keeps_recordings_without_long_silent_edges() {
        let segments = [segment(3.0, 10.0), segment(12.0, 50.0)];
        assert_eq!(speech_window(&segments, Some(55.0)), None);
        assert_eq!(speech_window(&[], Some(600.0)), None);
    }

    #[test]
    fn cuts_long_silence_on_either_edge_with_a_margin() {
        let segments = [segment(120.0, 130.0), segment(140.0, 300.0)];
        assert_eq!(speech_window(&segments, Some(900.0)), Some((118.0, 302.0)));
        assert_eq!(speech_window(&segments, Some(303.0)), Some((118.0, f64::INFINITY)));
        assert_eq!(speech_window(&segments, None), Some((118.0, f64::INFINITY)));
        let early = [segment(1.0, 300.0)];
        assert_eq!(speech_window(&early, Some(900.0)), Some((0.0, 302.0)));
    }

    #[test]
    fn shifting_keeps_segments_on_the_new_timeline() {
        let mut segments = [segment(120.0, 130.0), segment(117.5, 119.0)];
        shift_segments(&mut segments, 118.0);
        assert_eq!(segments[0].audio_start_time, 2.0);
        assert_eq!(segments[0].audio_end_time, 12.0);
        assert_eq!(segments[1].audio_start_time, 0.0);
        assert_eq!(segments[1].audio_end_time, 1.0);
    }
}
