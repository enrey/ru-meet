//! Human-readable transcript exports that sit next to `transcripts.json` in a
//! meeting folder.
//!
//! * `transcript.md` - one `HH:MM:SS - text` line per transcribed phrase.
//! * `transcript_with_speakers.md` - the same phrases as a dialogue, where
//!   consecutive phrases from the same speaker are merged into a single turn so
//!   the speaker is never repeated until somebody else talks. Written only once
//!   speaker labels exist (after diarization, or after a speaker rename).

use anyhow::{Context, Result};
use log::warn;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};

use crate::database::repositories::{
    meeting::MeetingsRepository, transcript::TranscriptsRepository,
};

pub const TRANSCRIPT_MARKDOWN_FILE: &str = "transcript.md";
pub const TRANSCRIPT_WITH_SPEAKERS_FILE: &str = "transcript_with_speakers.md";
const UNKNOWN_SPEAKER: &str = "Unknown speaker";

/// One transcribed phrase, in the shape both exports need.
#[derive(Debug, Clone)]
pub struct ExportLine {
    pub start_seconds: Option<f64>,
    pub text: String,
    pub speaker: Option<String>,
}

fn speaker_name(line: &ExportLine) -> Option<&str> {
    line.speaker
        .as_deref()
        .map(str::trim)
        .filter(|speaker| !speaker.is_empty())
}

pub fn format_timecode(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.floor() as u64
    } else {
        0
    };
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total % 3600) / 60,
        total % 60
    )
}

pub fn render_transcript_markdown(lines: &[ExportLine]) -> String {
    let mut rendered = String::new();
    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        rendered.push_str(&format_timecode(line.start_seconds.unwrap_or(0.0)));
        rendered.push_str(" - ");
        rendered.push_str(text);
        rendered.push('\n');
    }
    rendered
}

/// `None` when nothing carries a speaker label yet: an all-"Unknown speaker"
/// file is noise, and diarization may simply still be pending.
pub fn render_transcript_with_speakers_markdown(lines: &[ExportLine]) -> Option<String> {
    if !lines.iter().any(|line| speaker_name(line).is_some()) {
        return None;
    }

    let mut turns: Vec<(String, String)> = Vec::new();
    for line in lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        let speaker = speaker_name(line).unwrap_or(UNKNOWN_SPEAKER).to_string();
        match turns.last_mut() {
            Some((last_speaker, body)) if *last_speaker == speaker => {
                body.push(' ');
                body.push_str(text);
            }
            _ => turns.push((speaker, text.to_string())),
        }
    }

    if turns.is_empty() {
        return None;
    }
    Some(
        turns
            .iter()
            .map(|(speaker, body)| format!("{speaker}: {body}\n"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn write_atomic(folder: &Path, file_name: &str, contents: &str) -> Result<()> {
    let target = folder.join(file_name);
    let temp = folder.join(format!(".{file_name}.tmp"));
    std::fs::write(&temp, contents)
        .with_context(|| format!("Failed to write {}", temp.display()))?;
    std::fs::rename(&temp, &target)
        .with_context(|| format!("Failed to move transcript export into {}", target.display()))?;
    Ok(())
}

pub fn write_transcript_exports(folder: &Path, lines: &[ExportLine]) -> Result<()> {
    write_atomic(
        folder,
        TRANSCRIPT_MARKDOWN_FILE,
        &render_transcript_markdown(lines),
    )?;
    if let Some(dialogue) = render_transcript_with_speakers_markdown(lines) {
        write_atomic(folder, TRANSCRIPT_WITH_SPEAKERS_FILE, &dialogue)?;
    }
    Ok(())
}

/// Exports are a convenience artifact: never fail the transcription, recording
/// or rename that produced them.
pub fn write_transcript_exports_logged(folder: &Path, lines: &[ExportLine]) {
    if let Err(error) = write_transcript_exports(folder, lines) {
        warn!("Failed to write transcript markdown exports: {error:#}");
    }
}

async fn meeting_folder(pool: &SqlitePool, meeting_id: &str) -> Result<Option<PathBuf>> {
    let Some(meeting) = MeetingsRepository::get_meeting_metadata(pool, meeting_id).await? else {
        return Ok(None);
    };
    let Some(folder) = meeting.folder_path.map(PathBuf::from) else {
        return Ok(None);
    };
    Ok(folder.is_dir().then_some(folder))
}

/// Regenerate both exports for a meeting from the database, which is the source
/// of truth once a meeting is saved (diarization reruns, speaker renames).
pub async fn export_meeting_transcripts(pool: &SqlitePool, meeting_id: &str) -> Result<()> {
    let Some(folder) = meeting_folder(pool, meeting_id).await? else {
        return Ok(());
    };

    let lines = TranscriptsRepository::get_export_lines(pool, meeting_id)
        .await?
        .into_iter()
        .map(|(start_seconds, text, speaker)| ExportLine {
            start_seconds,
            text,
            speaker,
        })
        .collect::<Vec<_>>();

    write_transcript_exports(&folder, &lines)
}

pub async fn export_meeting_transcripts_logged(pool: &SqlitePool, meeting_id: &str) {
    if let Err(error) = export_meeting_transcripts(pool, meeting_id).await {
        warn!("Failed to export transcripts for meeting {meeting_id}: {error:#}");
    }
}

/// Give meetings recorded before these exports existed their files the first
/// time they are opened, instead of only after the next rename or rerun.
pub async fn export_meeting_transcripts_if_missing(pool: &SqlitePool, meeting_id: &str) {
    match meeting_folder(pool, meeting_id).await {
        Ok(Some(folder)) if !folder.join(TRANSCRIPT_MARKDOWN_FILE).exists() => {
            export_meeting_transcripts_logged(pool, meeting_id).await;
        }
        Ok(_) => {}
        Err(error) => warn!("Could not locate meeting folder for {meeting_id}: {error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start: f64, text: &str, speaker: Option<&str>) -> ExportLine {
        ExportLine {
            start_seconds: Some(start),
            text: text.to_string(),
            speaker: speaker.map(str::to_string),
        }
    }

    #[test]
    fn timecodes_roll_over_into_hours() {
        assert_eq!(format_timecode(0.0), "00:00:00");
        assert_eq!(format_timecode(83.7), "00:01:23");
        assert_eq!(format_timecode(3671.0), "01:01:11");
        assert_eq!(format_timecode(f64::NAN), "00:00:00");
    }

    #[test]
    fn plain_markdown_skips_empty_phrases() {
        let lines = vec![
            line(0.0, "Hello", None),
            line(5.0, "   ", None),
            line(83.0, "  World  ", None),
        ];
        assert_eq!(
            render_transcript_markdown(&lines),
            "00:00:00 - Hello\n00:01:23 - World\n"
        );
    }

    #[test]
    fn dialogue_merges_consecutive_turns_of_one_speaker() {
        let lines = vec![
            line(0.0, "Hello", Some("Speaker 1")),
            line(2.0, "how are you", Some("Speaker 1")),
            line(4.0, "Fine", Some("Speaker 2")),
            line(6.0, "and you", Some("Speaker 2")),
            line(8.0, "Great", Some("Speaker 1")),
        ];
        assert_eq!(
            render_transcript_with_speakers_markdown(&lines).unwrap(),
            "Speaker 1: Hello how are you\n\nSpeaker 2: Fine and you\n\nSpeaker 1: Great\n"
        );
    }

    #[test]
    fn dialogue_labels_gaps_but_needs_at_least_one_speaker() {
        assert!(render_transcript_with_speakers_markdown(&[
            line(0.0, "Hello", None),
            line(2.0, "World", Some("   ")),
        ])
        .is_none());

        assert_eq!(
            render_transcript_with_speakers_markdown(&[
                line(0.0, "Hello", None),
                line(2.0, "World", Some("Speaker 1")),
            ])
            .unwrap(),
            "Unknown speaker: Hello\n\nSpeaker 1: World\n"
        );
    }
}
