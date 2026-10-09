use crate::api::{MeetingDetails, MeetingTranscript};
use crate::database::models::{MeetingListRow, MeetingModel, Transcript};
use chrono::Utc;
use sqlx::{Connection, Error as SqlxError, SqliteConnection, SqlitePool};
use tracing::{error, info};

pub struct MeetingsRepository;

/// Longest preview line shown under a meeting title in the library.
const PREVIEW_MAX_CHARS: usize = 220;

impl MeetingsRepository {
    /// Meetings newest first, with the aggregates the library list shows.
    /// Each child table is scanned once via GROUP BY rather than per meeting.
    pub async fn get_meeting_list(pool: &SqlitePool) -> Result<Vec<MeetingListRow>, SqlxError> {
        sqlx::query_as::<_, MeetingListRow>(
            r#"
            SELECT
                m.id,
                m.title,
                m.created_at,
                m.folder_path,
                durations.duration_seconds,
                COALESCE(durations.transcript_count, 0) AS transcript_count,
                COALESCE(speakers.speaker_count, 0) AS speaker_count,
                LOWER(s.status) AS summary_status,
                s.error AS summary_error,
                CASE WHEN json_valid(s.result) THEN json_extract(s.result, '$.markdown') END
                    AS summary_markdown,
                firsts.transcript AS first_transcript
            FROM meetings m
            LEFT JOIN (
                SELECT meeting_id, MAX(audio_end_time) AS duration_seconds,
                    COUNT(*) AS transcript_count
                FROM transcripts GROUP BY meeting_id
            ) durations ON durations.meeting_id = m.id
            LEFT JOIN (
                -- SQLite returns the bare column from the row that holds MIN().
                SELECT meeting_id, transcript, MIN(COALESCE(audio_start_time, 0))
                FROM transcripts GROUP BY meeting_id
            ) firsts ON firsts.meeting_id = m.id
            LEFT JOIN (
                SELECT meeting_id, COUNT(DISTINCT speaker) AS speaker_count
                FROM diarization_turns GROUP BY meeting_id
            ) speakers ON speakers.meeting_id = m.id
            LEFT JOIN summary_processes s ON s.meeting_id = m.id
            ORDER BY m.created_at DESC
            "#,
        )
        .fetch_all(pool)
        .await
    }

    pub async fn delete_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        match delete_meeting_with_transaction(&mut transaction, meeting_id).await {
            Ok(success) => {
                if success {
                    transaction.commit().await?;
                    info!(
                        "Successfully deleted meeting {} and all associated data",
                        meeting_id
                    );
                    Ok(true)
                } else {
                    transaction.rollback().await?;
                    Ok(false)
                }
            }
            Err(e) => {
                let _ = transaction.rollback().await;
                error!("Failed to delete meeting {}: {}", meeting_id, e);
                Err(e)
            }
        }
    }

    pub async fn get_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingDetails>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        // Get meeting details
        let meeting: Option<MeetingModel> = sqlx::query_as(
            "SELECT id, title, created_at, updated_at, folder_path FROM meetings WHERE id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(&mut *transaction)
        .await?;

        if meeting.is_none() {
            transaction.rollback().await?;
            return Err(SqlxError::RowNotFound);
        }

        if let Some(meeting) = meeting {
            // Get all transcripts for this meeting
            let transcripts =
                sqlx::query_as::<_, Transcript>("SELECT * FROM transcripts WHERE meeting_id = ?")
                    .bind(meeting_id)
                    .fetch_all(&mut *transaction)
                    .await?;

            transaction.commit().await?;

            // Convert Transcript to MeetingTranscript
            let meeting_transcripts = transcripts
                .into_iter()
                .map(|t| MeetingTranscript {
                    id: t.id,
                    text: t.transcript,
                    timestamp: t.timestamp,
                    audio_start_time: t.audio_start_time,
                    audio_end_time: t.audio_end_time,
                    duration: t.duration,
                    speaker: t.speaker,
                })
                .collect::<Vec<_>>();

            Ok(Some(MeetingDetails {
                id: meeting.id,
                title: meeting.title,
                created_at: meeting.created_at.0.to_rfc3339(),
                updated_at: meeting.updated_at.0.to_rfc3339(),
                transcripts: meeting_transcripts,
            }))
        } else {
            transaction.rollback().await?;
            Ok(None)
        }
    }

    /// Get meeting metadata without transcripts (for pagination)
    pub async fn get_meeting_metadata(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Option<MeetingModel>, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let meeting: Option<MeetingModel> = sqlx::query_as(
            "SELECT id, title, created_at, updated_at, folder_path FROM meetings WHERE id = ?",
        )
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?;

        Ok(meeting)
    }

    /// Get meeting transcripts with pagination support
    pub async fn get_meeting_transcripts_paginated(
        pool: &SqlitePool,
        meeting_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<Transcript>, i64), SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        // Get total count of transcripts for this meeting
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_one(pool)
            .await?;

        // Get paginated transcripts ordered by audio_start_time
        let transcripts = sqlx::query_as::<_, Transcript>(
            "SELECT * FROM transcripts
             WHERE meeting_id = ?
             ORDER BY audio_start_time ASC, id ASC
             LIMIT ? OFFSET ?",
        )
        .bind(meeting_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok((transcripts, total.0))
    }

    pub async fn update_meeting_title(
        pool: &SqlitePool,
        meeting_id: &str,
        new_title: &str,
    ) -> Result<bool, SqlxError> {
        if meeting_id.trim().is_empty() {
            return Err(SqlxError::Protocol(
                "meeting_id cannot be empty".to_string(),
            ));
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now().naive_utc();

        let rows_affected =
            sqlx::query("UPDATE meetings SET title = ?, updated_at = ? WHERE id = ?")
                .bind(new_title)
                .bind(now)
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?;
        if rows_affected.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn update_meeting_name(
        pool: &SqlitePool,
        meeting_id: &str,
        new_title: &str,
    ) -> Result<bool, SqlxError> {
        let mut transaction = pool.begin().await?;
        let now = Utc::now();

        // Update meetings table
        let meeting_update =
            sqlx::query("UPDATE meetings SET title = ?, updated_at = ? WHERE id = ?")
                .bind(new_title)
                .bind(now)
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?;

        if meeting_update.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false); // Meeting not found
        }

        // Update transcript_chunks table
        sqlx::query("UPDATE transcript_chunks SET meeting_name = ? WHERE meeting_id = ?")
            .bind(new_title)
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(true)
    }
}

async fn delete_meeting_with_transaction(
    transaction: &mut SqliteConnection,
    meeting_id: &str,
) -> Result<bool, SqlxError> {
    // Check if meeting exists
    let meeting_exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(&mut *transaction)
        .await?;

    if meeting_exists.is_none() {
        error!("Meeting {} not found for deletion", meeting_id);
        return Ok(false);
    }

    // Delete from related tables in proper order
    // 1. Delete from transcript_chunks
    sqlx::query("DELETE FROM transcript_chunks WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 2. Delete from summary_processes
    sqlx::query("DELETE FROM summary_processes WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 3. Delete from transcripts
    sqlx::query("DELETE FROM transcripts WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    // 4. Finally, delete the meeting
    let result = sqlx::query("DELETE FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

    Ok(result.rows_affected() > 0)
}

/// One plain-text line for the library list: the first prose of the summary
/// (headings and markdown markup dropped), else the opening of the transcript.
pub fn meeting_preview(summary_markdown: Option<&str>, first_transcript: Option<&str>) -> Option<String> {
    summary_markdown
        .and_then(summary_prose)
        .or_else(|| first_transcript.map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned))
        .map(|text| truncate_chars(&text, PREVIEW_MAX_CHARS))
}

fn summary_prose(markdown: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut len = 0;
    for line in markdown.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('|') || line.starts_with("---") {
            continue;
        }
        let line = strip_list_marker(line).replace("**", "").replace('`', "");
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        len += line.chars().count();
        parts.push(line.to_owned());
        if len >= PREVIEW_MAX_CHARS {
            break;
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// Drops a leading `- `, `* `, `> `, `1. `, `2) ` and `[ ]`/`[x]` checkbox.
fn strip_list_marker(line: &str) -> &str {
    let mut rest = line;
    for marker in ["- ", "* ", "+ ", "> "] {
        if let Some(stripped) = rest.strip_prefix(marker) {
            rest = stripped.trim_start();
            break;
        }
    }
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 {
        let after = &rest[digits..];
        if let Some(stripped) = after.strip_prefix(". ").or_else(|| after.strip_prefix(") ")) {
            rest = stripped.trim_start();
        }
    }
    for checkbox in ["[ ] ", "[x] ", "[X] "] {
        if let Some(stripped) = rest.strip_prefix(checkbox) {
            return stripped.trim_start();
        }
    }
    rest
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_skips_headings_and_markup() {
        let markdown = "# Сводка\n\n## Решения\n- **Релиз** переносится\n1. Диаризация `polyvoice`\n";
        assert_eq!(
            meeting_preview(Some(markdown), Some("ignored")).as_deref(),
            Some("Релиз переносится; Диаризация polyvoice")
        );
        assert_eq!(
            meeting_preview(Some("- [x] xml экспорт\n2026 год"), None).as_deref(),
            Some("xml экспорт; 2026 год")
        );
    }

    #[test]
    fn preview_falls_back_to_transcript() {
        assert_eq!(meeting_preview(Some("# Only heading"), Some("  привет  ")).as_deref(), Some("привет"));
        assert_eq!(meeting_preview(None, Some("   ")), None);
    }

    #[test]
    fn preview_truncates_on_char_boundary() {
        let long = "я".repeat(PREVIEW_MAX_CHARS + 10);
        let preview = meeting_preview(None, Some(&long)).unwrap();
        assert_eq!(preview.chars().count(), PREVIEW_MAX_CHARS + 1);
        assert!(preview.ends_with('…'));
    }

    #[tokio::test]
    async fn meeting_list_aggregates_child_tables() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        for ddl in [
            "CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT, created_at TEXT, updated_at TEXT, folder_path TEXT)",
            "CREATE TABLE transcripts (id TEXT, meeting_id TEXT, transcript TEXT, audio_start_time REAL, audio_end_time REAL)",
            "CREATE TABLE diarization_turns (meeting_id TEXT, start_time REAL, end_time REAL, speaker TEXT)",
            "CREATE TABLE summary_processes (meeting_id TEXT PRIMARY KEY, status TEXT, result TEXT, error TEXT)",
            "INSERT INTO meetings VALUES ('a', 'Old', '2026-10-01T10:00:00Z', '2026-10-01T10:00:00Z', NULL)",
            "INSERT INTO meetings VALUES ('b', 'New', '2026-10-02T10:00:00Z', '2026-10-02T10:00:00Z', NULL)",
            "INSERT INTO transcripts VALUES ('1', 'b', 'second', 5.0, 61.5), ('2', 'b', 'first', 0.0, 4.0)",
            "INSERT INTO diarization_turns VALUES ('b', 0, 1, 'S1'), ('b', 1, 2, 'S2'), ('b', 2, 3, 'S1')",
            "INSERT INTO summary_processes VALUES ('b', 'COMPLETED', '{\"markdown\":\"Итог\"}', NULL), ('a', 'FAILED', NULL, 'boom')",
        ] {
            sqlx::query(ddl).execute(&pool).await.unwrap();
        }

        let rows = MeetingsRepository::get_meeting_list(&pool).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        let new = &rows[0];
        assert_eq!(new.duration_seconds, Some(61.5));
        assert_eq!(new.speaker_count, 2);
        assert_eq!(new.summary_status.as_deref(), Some("completed"));
        assert_eq!(new.summary_markdown.as_deref(), Some("Итог"));
        assert_eq!(new.first_transcript.as_deref(), Some("first"));
        assert_eq!(new.transcript_count, 2);
        let old = &rows[1];
        assert_eq!((old.duration_seconds, old.speaker_count), (None, 0));
        assert_eq!(old.transcript_count, 0);
        assert_eq!(old.summary_status.as_deref(), Some("failed"));
        assert_eq!(old.summary_error.as_deref(), Some("boom"));
        assert_eq!(old.summary_markdown, None);
    }
}
