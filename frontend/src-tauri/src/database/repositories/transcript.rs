use crate::api::{TranscriptSearchResult, TranscriptSegment};
use crate::audio::diarization::SpeakerTurn;
use chrono::Utc;
use serde::Deserialize;
use sqlx::{Connection, Error as SqlxError, SqlitePool};
use tracing::{error, info};
use uuid::Uuid;

pub struct TranscriptsRepository;

#[derive(Debug, Deserialize)]
pub struct SpeakerLabelUpdate {
    #[serde(alias = "audioStartTime")]
    pub audio_start_time: f64,
    #[serde(alias = "audioEndTime")]
    pub audio_end_time: f64,
    pub speaker: String,
}

async fn replace_speaker_turns(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    meeting_id: &str,
    turns: &[SpeakerTurn],
) -> Result<(), SqlxError> {
    sqlx::query("DELETE FROM diarization_turns WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut **transaction)
        .await?;
    for turn in turns {
        if !turn.start.is_finite()
            || !turn.end.is_finite()
            || turn.start < 0.0
            || turn.end <= turn.start
        {
            continue;
        }
        sqlx::query("INSERT INTO diarization_turns (meeting_id, start_time, end_time, speaker) VALUES (?, ?, ?, ?)")
            .bind(meeting_id)
            .bind(turn.start)
            .bind(turn.end)
            .bind(&turn.speaker)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

impl TranscriptsRepository {
    /// Find the phrase at the selected speaker/time and its index in the
    /// transcript's stable pagination order. A nearby phrase is used when the
    /// diarizer's speech interval falls between transcription segments.
    pub async fn find_transcript_at_time(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker: &str,
        time: f64,
    ) -> Result<Option<(String, i64)>, SqlxError> {
        let selected = sqlx::query_as::<_, (String, f64)>(
            "SELECT id, audio_start_time FROM transcripts
             WHERE meeting_id = ? AND audio_start_time IS NOT NULL
             ORDER BY
               CASE
                 WHEN speaker = ? AND audio_start_time <= ? AND audio_end_time >= ? THEN 0
                 WHEN audio_start_time <= ? AND audio_end_time >= ? THEN 1
                 WHEN speaker = ? THEN 2
                 ELSE 3
               END,
               ABS(audio_start_time - ?), id
             LIMIT 1",
        )
        .bind(meeting_id)
        .bind(speaker)
        .bind(time)
        .bind(time)
        .bind(time)
        .bind(time)
        .bind(speaker)
        .bind(time)
        .fetch_optional(pool)
        .await?;
        let Some((id, start)) = selected else {
            return Ok(None);
        };
        let (offset,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM transcripts
             WHERE meeting_id = ? AND (audio_start_time IS NULL
               OR audio_start_time < ? OR (audio_start_time = ? AND id < ?))",
        )
        .bind(meeting_id)
        .bind(start)
        .bind(start)
        .bind(&id)
        .fetch_one(pool)
        .await?;
        Ok(Some((id, offset)))
    }

    /// Saves a new meeting and its associated transcript segments.
    /// This function uses a transaction to ensure that either both the meeting
    /// and all its transcripts are saved, or none of them are.
    pub async fn save_transcript(
        pool: &SqlitePool,
        meeting_title: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now();

        // 1. Create the new meeting
        let result = sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(meeting_title)
        .bind(now)
        .bind(now)
        .bind(&folder_path)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = result {
            error!("Failed to create meeting '{}': {}", meeting_title, e);
            transaction.rollback().await?;
            return Err(e);
        }

        info!("Successfully created meeting with id: {}", meeting_id);

        // 2. Save each transcript segment with audio timing fields
        for segment in transcripts {
            let transcript_id = format!("transcript-{}", Uuid::new_v4());
            let result = sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&transcript_id)
            .bind(&meeting_id)
            .bind(&segment.text)
            .bind(&segment.timestamp)
            .bind(segment.audio_start_time)
            .bind(segment.audio_end_time)
            .bind(segment.duration)
            .bind(&segment.speaker)
            .execute(&mut *transaction)
            .await;

            if let Err(e) = result {
                error!(
                    "Failed to save transcript segment for meeting {}: {}",
                    meeting_id, e
                );
                transaction.rollback().await?;
                return Err(e);
            }
        }

        info!(
            "Successfully saved {} transcript segments for meeting {}",
            transcripts.len(),
            meeting_id
        );

        // Commit the transaction
        transaction.commit().await?;

        Ok(meeting_id)
    }

    /// Persists labels produced by the post-save diarization worker. Matching
    /// by recording-relative bounds is stable across frontend and SQLite data.
    pub async fn update_speakers(
        pool: &SqlitePool,
        meeting_id: &str,
        labels: &[SpeakerLabelUpdate],
        turns: &[SpeakerTurn],
    ) -> Result<(), SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        sqlx::query("UPDATE transcripts SET speaker = NULL WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;

        for label in labels {
            sqlx::query(
                "UPDATE transcripts
                 SET speaker = ?
                 WHERE meeting_id = ? AND audio_start_time = ? AND audio_end_time = ?",
            )
            .bind(&label.speaker)
            .bind(meeting_id)
            .bind(label.audio_start_time)
            .bind(label.audio_end_time)
            .execute(&mut *transaction)
            .await?;
        }

        replace_speaker_turns(&mut transaction, meeting_id, turns).await?;
        transaction.commit().await
    }

    pub async fn apply_speaker_turns(
        pool: &SqlitePool,
        meeting_id: &str,
        turns: &[SpeakerTurn],
    ) -> Result<(), SqlxError> {
        let segments = sqlx::query_as::<_, (String, Option<f64>, Option<f64>)>(
            "SELECT id, audio_start_time, audio_end_time FROM transcripts WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        sqlx::query("UPDATE transcripts SET speaker = NULL WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *transaction)
            .await?;

        for (id, start, end) in segments {
            let (Some(start), Some(end)) = (start, end) else {
                continue;
            };
            let speaker = turns
                .iter()
                .filter_map(|turn| {
                    let overlap = (end.min(turn.end) - start.max(turn.start)).max(0.0);
                    (overlap > 0.0).then_some((overlap, &turn.speaker))
                })
                .max_by(|left, right| left.0.total_cmp(&right.0))
                .map(|(_, speaker)| speaker);
            if let Some(speaker) = speaker {
                sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                    .bind(speaker)
                    .bind(id)
                    .execute(&mut *transaction)
                    .await?;
            }
        }

        replace_speaker_turns(&mut transaction, meeting_id, turns).await?;
        transaction.commit().await
    }

    /// Phrases in transcript order, as the markdown exports need them.
    pub async fn get_export_lines(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<(Option<f64>, String, Option<String>)>, SqlxError> {
        sqlx::query_as::<_, (Option<f64>, String, Option<String>)>(
            "SELECT audio_start_time, transcript, speaker FROM transcripts \
             WHERE meeting_id = ? ORDER BY audio_start_time ASC, id ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    /// Meetings diarized through the live-recording path can end up with speaker
    /// turns but unlabelled transcript rows: those labels were matched on exact
    /// float timestamps supplied by the frontend, which silently matches nothing
    /// when the frontend's copy of a segment has drifted. Re-derive the missing
    /// labels from the stored turns by overlap, so the speaker timeline and the
    /// transcript agree - and so renaming a speaker reaches both.
    ///
    /// Returns the number of transcript rows labelled. Meetings that already
    /// carry any label are left alone, so this never fights a manual rename.
    pub async fn backfill_speakers_from_turns(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<u64, SqlxError> {
        let labelled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcripts \
             WHERE meeting_id = ? AND speaker IS NOT NULL AND TRIM(speaker) != ''",
        )
        .bind(meeting_id)
        .fetch_one(pool)
        .await?;
        if labelled > 0 {
            return Ok(0);
        }

        let turns = sqlx::query_as::<_, (f64, f64, String)>(
            "SELECT start_time, end_time, speaker FROM diarization_turns WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        if turns.is_empty() {
            return Ok(0);
        }

        let segments = sqlx::query_as::<_, (String, Option<f64>, Option<f64>)>(
            "SELECT id, audio_start_time, audio_end_time FROM transcripts WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;
        let mut updated = 0u64;
        for (id, start, end) in segments {
            let (Some(start), Some(end)) = (start, end) else {
                continue;
            };
            let speaker = turns
                .iter()
                .filter_map(|(turn_start, turn_end, speaker)| {
                    let overlap = (end.min(*turn_end) - start.max(*turn_start)).max(0.0);
                    (overlap > 0.0).then_some((overlap, speaker))
                })
                .max_by(|left, right| left.0.total_cmp(&right.0))
                .map(|(_, speaker)| speaker);
            if let Some(speaker) = speaker {
                sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                    .bind(speaker)
                    .bind(id)
                    .execute(&mut *transaction)
                    .await?;
                updated += 1;
            }
        }
        transaction.commit().await?;
        Ok(updated)
    }

    pub async fn get_speaker_turns(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerTurn>, SqlxError> {
        let rows = sqlx::query_as::<_, (f64, f64, String)>(
            "SELECT start_time, end_time, speaker FROM diarization_turns WHERE meeting_id = ? ORDER BY start_time, end_time",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        if !rows.is_empty() {
            return Ok(rows
                .into_iter()
                .map(|(start, end, speaker)| SpeakerTurn {
                    start,
                    end,
                    speaker,
                })
                .collect());
        }

        // Meetings diarized before this migration only have labels on their
        // transcript segments. Use those intervals until diarization is rerun.
        let legacy = sqlx::query_as::<_, (f64, f64, String)>(
            "SELECT audio_start_time, audio_end_time, speaker FROM transcripts \
             WHERE meeting_id = ? AND speaker IS NOT NULL AND TRIM(speaker) != '' \
             AND audio_start_time IS NOT NULL AND audio_end_time > audio_start_time \
             ORDER BY audio_start_time, audio_end_time",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(legacy
            .into_iter()
            .map(|(start, end, speaker)| SpeakerTurn {
                start,
                end,
                speaker,
            })
            .collect())
    }

    /// Rename a speaker, merging into an existing one when the name is taken.
    ///
    /// Diarization routinely splits one person into several speakers - a voice
    /// that changes character partway through a recording is enough - so
    /// renaming onto an existing name is how the user says "these are the same
    /// person". Returns whether such a merge happened, so the caller can say so.
    /// Merging cannot be undone short of re-running diarization; the UI asks
    /// first.
    pub async fn rename_speaker(
        pool: &SqlitePool,
        meeting_id: &str,
        old_name: &str,
        new_name: &str,
    ) -> Result<bool, String> {
        let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
        let old_count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM diarization_turns WHERE meeting_id = ? AND speaker = ?) + \
                    (SELECT COUNT(*) FROM transcripts WHERE meeting_id = ? AND speaker = ?)",
        )
        .bind(meeting_id).bind(old_name).bind(meeting_id).bind(old_name)
        .fetch_one(&mut *transaction).await.map_err(|error| error.to_string())?;
        if old_count == 0 {
            return Err("Speaker was not found in this meeting".into());
        }
        let existing_count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM diarization_turns WHERE meeting_id = ? AND speaker = ?) + \
                    (SELECT COUNT(*) FROM transcripts WHERE meeting_id = ? AND speaker = ?)",
        )
        .bind(meeting_id).bind(new_name).bind(meeting_id).bind(new_name)
        .fetch_one(&mut *transaction).await.map_err(|error| error.to_string())?;
        // Not an error: the two speakers simply become one. Both statements
        // below are plain relabels, so no constraint stands in the way.
        let merged = existing_count > 0;
        sqlx::query(
            "UPDATE diarization_turns SET speaker = ? WHERE meeting_id = ? AND speaker = ?",
        )
        .bind(new_name)
        .bind(meeting_id)
        .bind(old_name)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
            .bind(new_name)
            .bind(meeting_id)
            .bind(old_name)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        Ok(merged)
    }

    /// Searches every meeting's transcript lines for `query`.
    ///
    /// Matching is done in Rust: SQLite's `LOWER`/`LIKE` only fold ASCII, so
    /// a Cyrillic query would otherwise be case-sensitive.
    pub async fn search_transcripts(
        pool: &SqlitePool,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, SqlxError> {
        use futures_util::TryStreamExt;

        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT m.id, m.title, t.transcript, t.timestamp
             FROM meetings m
             JOIN transcripts t ON m.id = t.meeting_id",
        )
        .fetch(pool);
        let mut results = Vec::new();
        while let Some((id, title, transcript, timestamp)) = rows.try_next().await? {
            if let Some(match_context) = Self::get_match_context(&transcript, &needle) {
                results.push(TranscriptSearchResult {
                    id,
                    title,
                    match_context,
                    timestamp,
                });
            }
        }
        Ok(results)
    }

    /// Lines of one meeting containing `query`, as `(id, offset)` where
    /// `offset` is the line's position in the paginated transcript order
    /// (`audio_start_time ASC, id ASC`), so the UI can load the right page.
    pub async fn search_meeting_transcript(
        pool: &SqlitePool,
        meeting_id: &str,
        query: &str,
    ) -> Result<Vec<(String, i64)>, SqlxError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT id, transcript FROM transcripts
             WHERE meeting_id = ? ORDER BY audio_start_time ASC, id ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .enumerate()
            .filter(|(_, (_, text))| text.to_lowercase().contains(&needle))
            .map(|(offset, (id, _))| (id, offset as i64))
            .collect())
    }

    /// Up to 100 characters either side of the first case-insensitive match of
    /// the lowercase `needle`, or `None` when the text does not contain it.
    /// Works on characters, never bytes, so it cannot split a Cyrillic letter.
    fn get_match_context(transcript: &str, needle: &str) -> Option<String> {
        const AROUND: usize = 100;
        let chars: Vec<char> = transcript.chars().collect();
        let lower: Vec<char> = chars
            .iter()
            .map(|c| c.to_lowercase().next().unwrap_or(*c))
            .collect();
        let needle: Vec<char> = needle.chars().collect();
        let start = lower
            .windows(needle.len().max(1))
            .position(|window| window == needle.as_slice())?;
        let from = start.saturating_sub(AROUND);
        let to = (start + needle.len() + AROUND).min(chars.len());
        let mut context = String::new();
        if from > 0 {
            context.push_str("...");
        }
        context.extend(&chars[from..to]);
        if to < chars.len() {
            context.push_str("...");
        }
        Some(context)
    }
}

#[cfg(test)]
mod search_tests {
    use super::TranscriptsRepository;

    #[test]
    fn match_context_is_case_insensitive_for_cyrillic_and_char_safe() {
        let text = format!("{}Привет, мир{}", "я".repeat(150), "ю".repeat(150));
        let context = TranscriptsRepository::get_match_context(&text, "привет").unwrap();
        assert!(context.starts_with("...") && context.ends_with("..."));
        assert!(context.contains("Привет, мир"));
        assert_eq!(TranscriptsRepository::get_match_context("Hello", "bye"), None);
    }

    #[tokio::test]
    async fn meeting_search_returns_page_offsets_in_transcript_order() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE transcripts (id TEXT, meeting_id TEXT, transcript TEXT, audio_start_time REAL)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO transcripts VALUES
            ('c', 'm', 'Третий: ПРИВЕТ', 30.0), ('a', 'm', 'первый', 10.0),
            ('b', 'm', 'Второй привет', 20.0), ('x', 'other', 'привет', 5.0)")
            .execute(&pool).await.unwrap();
        let found = TranscriptsRepository::search_meeting_transcript(&pool, "m", "Привет").await.unwrap();
        assert_eq!(found, vec![("b".to_string(), 1), ("c".to_string(), 2)]);
    }
}
