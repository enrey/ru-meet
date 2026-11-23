use crate::api::{TranscriptSearchResult, TranscriptSegment};
use crate::audio::diarization::SpeakerTurn;
use crate::audio::word_timing::WordTiming;
use crate::database::repositories::voiceprint::VoiceprintRepository;
use chrono::Utc;
use serde::{Deserialize, Serialize};
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

#[derive(Serialize, Deserialize)]
struct DiarizationSource {
    text: String,
    start: f64,
    end: f64,
    words: Vec<WordTiming>,
}

#[derive(sqlx::FromRow)]
struct DiarizationRow {
    id: String,
    transcript: String,
    timestamp: String,
    audio_start_time: Option<f64>,
    audio_end_time: Option<f64>,
    diarization_source: Option<String>,
    diarization_parent: Option<String>,
    speaker_edited: bool,
    recording_source: Option<String>,
    term_corrections: Option<String>,
}

/// A tied turn at most this share of the longest tied one counts as an
/// interjection inside it and takes the word.
const INTERJECTION_SHARE: f64 = 0.5;

fn overlap(start: f64, end: f64, turn: &SpeakerTurn) -> f64 {
    (end.min(turn.end) - start.max(turn.start)).max(0.0)
}

fn segment_speaker(start: f64, end: f64, turns: &[SpeakerTurn]) -> Option<&str> {
    turns
        .iter()
        .map(|turn| (overlap(start, end, turn), turn.speaker.as_str()))
        .filter(|(amount, _)| *amount > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, speaker)| speaker)
}

impl DiarizationSource {
    fn is_usable(&self) -> bool {
        self.start.is_finite()
            && self.end.is_finite()
            && self.start >= 0.0
            && self.end > self.start
            && self.words.first().is_some_and(|word| word.byte_start == 0)
            && self.words.iter().all(|word| {
                word.byte_start < self.text.len()
                    && self.text.is_char_boundary(word.byte_start)
                    && word.start >= self.start
                    && word.end <= self.end + 1e-6
                    && word.end >= word.start
            })
            && self.words.windows(2).all(|pair| {
                pair[0].byte_start < pair[1].byte_start && pair[0].start <= pair[1].start
            })
            && (self.words.len() == 1 || self.words[0].start < self.words.last().unwrap().start)
    }

    fn split<'a>(&'a self, turns: &'a [SpeakerTurn]) -> Vec<(&'a str, f64, f64, Option<&'a str>)> {
        let fallback = segment_speaker(self.start, self.end, turns);
        let mut parts = Vec::new();
        let (mut byte, mut start, mut speaker) = (0, self.start, None);
        let mut spoken_end = start;
        for word in &self.words {
            // Zero-length token ranges can still locate a word. Treat the end
            // of a turn as exclusive at such a boundary.
            let score = |turn: &SpeakerTurn| {
                if word.start == word.end {
                    f64::from(turn.start <= word.start && word.start < turn.end)
                } else {
                    overlap(word.start, word.end, turn)
                }
            };
            let best = turns.iter().map(score).fold(0.0, f64::max);
            let next = if best > 0.0 {
                // On near ties a short interjection nested in a much longer
                // turn wins ("да" over the overlapped monologue); otherwise
                // keep the current speaker. For an initial tie use the
                // midpoint, then a deterministic speaker order.
                let midpoint = (word.start + word.end) / 2.0;
                let tied: Vec<&SpeakerTurn> = turns
                    .iter()
                    .filter(|turn| score(turn) > 0.0 && best - score(turn) <= 0.001)
                    .collect();
                let longest = tied
                    .iter()
                    .map(|turn| turn.end - turn.start)
                    .fold(0.0, f64::max);
                tied.into_iter()
                    .min_by_key(|turn| {
                        (
                            turn.end - turn.start > INTERJECTION_SHARE * longest,
                            speaker != Some(turn.speaker.as_str()),
                            !(turn.start <= midpoint && midpoint < turn.end),
                            turn.speaker.as_str(),
                        )
                    })
                    .map(|turn| turn.speaker.as_str())
            } else {
                speaker.or(fallback)
            };

            if word.byte_start == 0 {
                speaker = next;
            } else if next != speaker && word.start > start {
                parts.push((
                    &self.text[byte..word.byte_start],
                    start,
                    spoken_end.min(word.start),
                    speaker,
                ));
                (byte, start, speaker) = (word.byte_start, word.start, next);
                spoken_end = start;
            }
            spoken_end = spoken_end.max(word.end);
        }
        parts.push((&self.text[byte..], start, self.end, speaker));
        parts
    }
}

async fn replace_speaker_turns(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    meeting_id: &str,
    turns: &[SpeakerTurn],
) -> Result<Vec<SpeakerTurn>, SqlxError> {
    let renamed = sqlx::query_as::<_, (f64, f64, String, String)>(
        "SELECT start_time, end_time, original_speaker, speaker FROM diarization_turns
         WHERE meeting_id = ? AND original_speaker IS NOT NULL",
    )
    .bind(meeting_id)
    .fetch_all(&mut **transaction)
    .await?;
    let mut applied = Vec::new();
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
        let corrected = renamed
            .iter()
            .find(|(start, end, original, _)| {
                *start == turn.start && *end == turn.end && original == &turn.speaker
            })
            .map(|(_, _, _, speaker)| speaker);
        let speaker = corrected.unwrap_or(&turn.speaker);
        sqlx::query("INSERT INTO diarization_turns (meeting_id, start_time, end_time, speaker, original_speaker) VALUES (?, ?, ?, ?, ?)")
            .bind(meeting_id)
            .bind(turn.start)
            .bind(turn.end)
            .bind(speaker)
            .bind(corrected.map(|_| &turn.speaker))
            .execute(&mut **transaction)
            .await?;
        applied.push(SpeakerTurn {
            speaker: speaker.clone(),
            ..turn.clone()
        });
    }
    Ok(applied)
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
            let result =
                Self::insert_segment(&mut transaction, &meeting_id, &transcript_id, segment).await;

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

    pub(crate) async fn insert_segment(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        meeting_id: &str,
        id: &str,
        segment: &TranscriptSegment,
    ) -> Result<(), SqlxError> {
        let source = match (segment.audio_start_time, segment.audio_end_time) {
            (Some(start), Some(end)) if !segment.words.is_empty() => {
                let source = DiarizationSource {
                    text: segment.text.clone(),
                    start,
                    end,
                    words: segment.words.clone(),
                };
                source
                    .is_usable()
                    .then(|| {
                        serde_json::to_string(&source)
                            .map_err(|error| SqlxError::Encode(error.into()))
                    })
                    .transpose()?
            }
            _ => None,
        };
        let corrections = (!segment.term_corrections.is_empty())
            .then(|| serde_json::to_string(&segment.term_corrections))
            .transpose()
            .map_err(|error| SqlxError::Encode(error.into()))?;
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker, diarization_source, recording_source, term_corrections)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(id)
        .bind(meeting_id)
        .bind(&segment.text)
        .bind(&segment.timestamp)
        .bind(segment.audio_start_time)
        .bind(segment.audio_end_time)
        .bind(segment.duration)
        .bind(&segment.speaker)
        .bind(source)
        .bind(segment.recording_source.map(|source| source.as_str()))
        .bind(corrections)
        .execute(&mut **transaction)
        .await?;
        Ok(())
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

        sqlx::query(
            "UPDATE transcripts SET speaker = NULL WHERE meeting_id = ? AND speaker_edited = 0",
        )
        .bind(meeting_id)
        .execute(&mut *transaction)
        .await?;

        for label in labels {
            sqlx::query(
                "UPDATE transcripts
                 SET speaker = ?
                 WHERE meeting_id = ? AND audio_start_time = ? AND audio_end_time = ? AND speaker_edited = 0",
            )
            .bind(&label.speaker)
            .bind(meeting_id)
            .bind(label.audio_start_time)
            .bind(label.audio_end_time)
            .execute(&mut *transaction)
            .await?;
        }

        replace_speaker_turns(&mut transaction, meeting_id, turns).await?;
        VoiceprintRepository::reconcile_after_diarization(&mut transaction, meeting_id).await?;
        transaction.commit().await
    }

    pub async fn apply_speaker_turns(
        pool: &SqlitePool,
        meeting_id: &str,
        turns: &[SpeakerTurn],
    ) -> Result<(), SqlxError> {
        let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
        let turns = replace_speaker_turns(&mut transaction, meeting_id, turns).await?;
        Self::assign_speakers(&mut transaction, meeting_id, &turns).await?;
        VoiceprintRepository::reconcile_after_diarization(&mut transaction, meeting_id).await?;
        transaction.commit().await
    }

    /// Reuse the current speaker timeline when replacing a transcript from
    /// the same recording. In particular, carry over renamed speaker labels.
    pub(crate) async fn apply_stored_speaker_turns(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        meeting_id: &str,
    ) -> Result<bool, SqlxError> {
        let turns = sqlx::query_as::<_, (f64, f64, String)>(
            "SELECT start_time, end_time, speaker FROM diarization_turns WHERE meeting_id = ? ORDER BY start_time, end_time",
        )
        .bind(meeting_id)
        .fetch_all(&mut **transaction)
        .await?
        .into_iter()
        .map(|(start, end, speaker)| SpeakerTurn { start, end, speaker })
        .collect::<Vec<_>>();
        if turns.is_empty() {
            return Ok(false);
        }
        Self::assign_speakers(transaction, meeting_id, &turns).await?;
        Ok(true)
    }

    async fn assign_speakers(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        meeting_id: &str,
        turns: &[SpeakerTurn],
    ) -> Result<(), SqlxError> {
        let rows = sqlx::query_as::<_, DiarizationRow>(
            "SELECT id, transcript, timestamp, audio_start_time, audio_end_time, diarization_source,
                    diarization_parent, speaker_edited, recording_source, term_corrections FROM transcripts WHERE meeting_id = ? ORDER BY id",
        )
        .bind(meeting_id)
        .fetch_all(&mut **transaction)
        .await?;
        let mut children = std::collections::HashMap::<_, Vec<_>>::new();
        for row in &rows {
            if let Some(parent) = &row.diarization_parent {
                children.entry(parent.as_str()).or_default().push(row);
            }
        }

        for row in rows.iter().filter(|row| {
            row.diarization_parent.is_none()
                && !row.speaker_edited
                && row.recording_source.as_deref() != Some("microphone")
        }) {
            let Some(source) = row
                .diarization_source
                .as_deref()
                .and_then(|source| serde_json::from_str::<DiarizationSource>(source).ok())
                .filter(DiarizationSource::is_usable)
            else {
                let speaker = row
                    .audio_start_time
                    .zip(row.audio_end_time)
                    .and_then(|(start, end)| segment_speaker(start, end, turns));
                sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                    .bind(speaker)
                    .bind(&row.id)
                    .execute(&mut **transaction)
                    .await?;
                continue;
            };

            let siblings = children
                .get(row.id.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default();
            if siblings.iter().any(|child| child.speaker_edited)
                || !source.text.bytes().eq(std::iter::once(row)
                    .chain(siblings.iter().copied())
                    .flat_map(|part| part.transcript.bytes()))
            {
                // Keep edited text and manually assigned speakers as stored.
                continue;
            }

            let nearby: Vec<_> = turns
                .iter()
                .filter(|turn| overlap(source.start, source.end, turn) > 0.0)
                .cloned()
                .collect();
            let parts = source.split(&nearby);
            if !siblings.is_empty() {
                sqlx::query(
                    "DELETE FROM transcripts WHERE meeting_id = ? AND diarization_parent = ?",
                )
                .bind(meeting_id)
                .bind(&row.id)
                .execute(&mut **transaction)
                .await?;
            }
            for (index, (text, start, end, speaker)) in parts.into_iter().enumerate() {
                if index == 0 {
                    sqlx::query("UPDATE transcripts SET transcript = ?, audio_start_time = ?, audio_end_time = ?, duration = ?, speaker = ? WHERE id = ?")
                        .bind(text).bind(start).bind(end).bind(end - start).bind(speaker).bind(&row.id)
                        .execute(&mut **transaction)
                        .await?;
                } else {
                    sqlx::query(
                        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker, diarization_parent, recording_source, term_corrections)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
                    )
                    .bind(format!("{}-part-{index:08}", row.id))
                    .bind(meeting_id).bind(text).bind(&row.timestamp)
                    .bind(start).bind(end).bind(end - start).bind(speaker).bind(&row.id)
                    .bind(&row.recording_source).bind(&row.term_corrections)
                    .execute(&mut **transaction)
                    .await?;
                }
            }
        }

        Ok(())
    }

    pub(crate) async fn edited_rows(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        meeting_id: &str,
    ) -> Result<Vec<(String, Option<f64>, Option<f64>, Option<String>)>, SqlxError> {
        sqlx::query_as(
            "WITH edited_group AS (
                SELECT COALESCE(parent.diarization_parent, parent.id) AS root
                FROM transcripts AS parent WHERE parent.meeting_id = ?1
                AND (parent.speaker_edited = 1 OR (
                    parent.diarization_parent IS NULL
                    AND json_valid(parent.diarization_source)
                    AND json_extract(parent.diarization_source, '$.text') != (parent.transcript || COALESCE((
                        SELECT group_concat(child_text, '') FROM (
                            SELECT transcript AS child_text FROM transcripts
                            WHERE meeting_id = parent.meeting_id AND diarization_parent = parent.id ORDER BY id
                        )
                    ), ''))
                ))
            )
            SELECT id, audio_start_time, audio_end_time, recording_source FROM transcripts
            WHERE meeting_id = ?1 AND COALESCE(diarization_parent, id) IN (SELECT root FROM edited_group)"
        ).bind(meeting_id).fetch_all(&mut **tx).await
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
            "UPDATE diarization_turns SET speaker = ?, original_speaker = COALESCE(original_speaker, speaker) WHERE meeting_id = ? AND speaker = ?",
        )
        .bind(new_name)
        .bind(meeting_id)
        .bind(old_name)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
        sqlx::query("UPDATE transcripts SET speaker = ?, speaker_edited = 1 WHERE meeting_id = ? AND speaker = ?")
            .bind(new_name)
            .bind(meeting_id)
            .bind(old_name)
            .execute(&mut *transaction)
            .await
            .map_err(|error| error.to_string())?;
        VoiceprintRepository::follow_speaker_rename(
            &mut transaction,
            meeting_id,
            old_name,
            new_name,
        )
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
mod word_timing_tests {
    use super::*;
    use crate::database::repositories::meeting::MeetingsRepository;

    #[test]
    fn word_timing_ties_keep_context_and_points_use_half_open_turns() {
        let source = DiarizationSource {
            text: "Раз два три".to_owned(),
            start: 0.0,
            end: 3.0,
            words: [(0, 0.1, 0.5), (7, 0.8, 1.2), (14, 1.5, 1.5)]
                .into_iter()
                .map(|(byte_start, start, end)| WordTiming {
                    byte_start,
                    start,
                    end,
                })
                .collect(),
        };
        let turns = [
            SpeakerTurn {
                start: 0.0,
                end: 1.5,
                speaker: "А".into(),
            },
            SpeakerTurn {
                start: 0.8,
                end: 2.0,
                speaker: "Б".into(),
            },
        ];
        let parts = source.split(&turns);
        assert_eq!(
            parts,
            [
                ("Раз два ", 0.0, 1.2, Some("А")),
                ("три", 1.5, 3.0, Some("Б"))
            ]
        );
    }

    #[test]
    fn word_timing_ties_go_to_nested_interjection() {
        let source = DiarizationSource {
            text: "Значит да делаем".to_owned(),
            start: 0.0,
            end: 6.0,
            words: [(0, 0.5, 1.0), (13, 3.0, 3.3), (18, 4.5, 5.0)]
                .into_iter()
                .map(|(byte_start, start, end)| WordTiming {
                    byte_start,
                    start,
                    end,
                })
                .collect(),
        };
        let turns = [
            SpeakerTurn {
                start: 0.0,
                end: 6.0,
                speaker: "А".into(),
            },
            SpeakerTurn {
                start: 2.9,
                end: 3.5,
                speaker: "Б".into(),
            },
        ];
        let speakers: Vec<_> = source.split(&turns).into_iter().map(|part| part.3).collect();
        assert_eq!(speakers, [Some("А"), Some("Б"), Some("А")]);
    }

    #[tokio::test]
    async fn word_timing_splits_screen_rows_and_exports_and_preserves_manual_edits() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let folder = tempfile::tempdir().unwrap();
        let turn = |start, end, speaker: &str| SpeakerTurn {
            start,
            end,
            speaker: speaker.into(),
        };
        let text = "Начинаем? «Да». Тогда отправлю документы.";
        let words = ["Начинаем", "«Да»", "Тогда", "отправлю", "документы"]
            .into_iter()
            .zip([
                (12.1, 12.8),
                (13.0, 13.22),
                (13.4, 13.6),
                (13.7, 14.1),
                (14.2, 14.8),
            ])
            .map(|(word, (start, end))| WordTiming {
                byte_start: text.find(word).unwrap(),
                start,
                end,
            })
            .collect();
        let segment = |text: &str, start, end, words| TranscriptSegment {
            id: String::new(),
            text: text.into(),
            timestamp: "00:00:00".into(),
            audio_start_time: Some(start),
            audio_end_time: Some(end),
            duration: Some(end - start),
            speaker: None,
            words,
            recording_source: None,
            term_corrections: Vec::new(),
        };
        let meeting = TranscriptsRepository::save_transcript(
            &pool,
            "word timing",
            &[
                segment(text, 12.0, 15.0, words),
                segment("Старая строка", 20.0, 23.0, vec![]),
            ],
            Some(folder.path().to_string_lossy().into_owned()),
        )
        .await
        .unwrap();
        let turns = [
            turn(12.0, 13.0, "А"),
            turn(13.0, 13.25, "Б"),
            turn(13.25, 15.0, "А"),
            turn(20.0, 22.0, "А"),
            turn(22.0, 23.0, "Б"),
        ];
        let read = || async {
            MeetingsRepository::get_meeting_transcripts_paginated(&pool, &meeting, 10, 0)
                .await
                .unwrap()
                .0
                .into_iter()
                .map(|row| (row.id, row.transcript, row.speaker.unwrap()))
                .collect::<Vec<_>>()
        };
        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &turns)
            .await
            .unwrap();
        let first = read().await;
        assert_eq!(
            first
                .iter()
                .map(|(_, text, speaker)| (text.trim(), speaker.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Начинаем?", "А"),
                ("«Да».", "Б"),
                ("Тогда отправлю документы.", "А"),
                ("Старая строка", "А")
            ]
        );
        assert_eq!(
            first
                .iter()
                .take(3)
                .map(|(_, text, _)| text.as_str())
                .collect::<String>(),
            text
        );
        for _ in 0..2 {
            TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &turns)
                .await
                .unwrap();
            assert_eq!(read().await, first);
        }
        {
            let mut tx = pool.begin().await.unwrap();
            assert!(TranscriptsRepository::edited_rows(&mut tx, &meeting)
                .await
                .unwrap()
                .is_empty());
        }
        crate::audio::transcript_export::export_diarized_transcripts(&pool, &meeting)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(folder.path().join("transcript_with_speakers.md")).unwrap(),
            "А: Начинаем?\n\nБ: «Да».\n\nА: Тогда отправлю документы. Старая строка\n"
        );

        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &[turn(12.0, 23.0, "А")])
            .await
            .unwrap();
        assert_eq!(read().await.len(), 2);
        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &turns)
            .await
            .unwrap();
        assert_eq!(read().await, first);
        TranscriptsRepository::rename_speaker(&pool, &meeting, "Б", "Борис")
            .await
            .unwrap();
        sqlx::query("UPDATE transcripts SET transcript = ? WHERE id = ?")
            .bind("Исправленный вопрос? ")
            .bind(&first[0].0)
            .execute(&pool)
            .await
            .unwrap();
        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &turns)
            .await
            .unwrap();
        let edited = read().await;
        assert_eq!(edited[0].1, "Исправленный вопрос? ");
        assert_eq!(edited[1].2, "Борис");
        {
            let mut tx = pool.begin().await.unwrap();
            let protected = TranscriptsRepository::edited_rows(&mut tx, &meeting)
                .await
                .unwrap();
            assert_eq!(protected.len(), 3);
            assert!(protected.iter().any(|entry| entry.0 == edited[0].0));
        }
        assert_eq!(
            TranscriptsRepository::get_speaker_turns(&pool, &meeting)
                .await
                .unwrap()[1]
                .speaker,
            "Борис"
        );
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
        assert_eq!(
            TranscriptsRepository::get_match_context("Hello", "bye"),
            None
        );
    }

    #[tokio::test]
    async fn meeting_search_returns_page_offsets_in_transcript_order() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE transcripts (id TEXT, meeting_id TEXT, transcript TEXT, audio_start_time REAL)")
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO transcripts VALUES
            ('c', 'm', 'Третий: ПРИВЕТ', 30.0), ('a', 'm', 'первый', 10.0),
            ('b', 'm', 'Второй привет', 20.0), ('x', 'other', 'привет', 5.0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let found = TranscriptsRepository::search_meeting_transcript(&pool, "m", "Привет")
            .await
            .unwrap();
        assert_eq!(found, vec![("b".to_string(), 1), ("c".to_string(), 2)]);
    }
}
