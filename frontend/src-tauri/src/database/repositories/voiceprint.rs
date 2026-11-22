//! Storage of the local voiceprint library: profiles, their samples and the
//! links between meeting speakers and profiles.

use chrono::Utc;
use serde::Serialize;
use sqlx::{Error as SqlxError, SqlitePool};
use uuid::Uuid;

pub struct VoiceprintRepository;

pub const STATUS_SUGGESTED: &str = "suggested";
pub const STATUS_CONFIRMED: &str = "confirmed";
pub const STATUS_REJECTED: &str = "rejected";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSampleInfo {
    pub id: String,
    pub meeting_id: Option<String>,
    pub meeting_title: Option<String>,
    pub start: f64,
    pub end: f64,
    /// False for samples made by an older extractor: they are kept, but no
    /// longer take part in matching.
    pub current: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfileInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub samples: Vec<VoiceSampleInfo>,
    /// Meetings where this profile was confirmed for a speaker.
    pub confirmed_meetings: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerProfileLink {
    pub speaker: String,
    pub profile_id: String,
    pub profile_name: String,
    pub status: String,
    pub similarity: Option<f64>,
}

/// Embeddings of one profile made by the given extractor.
pub struct ProfileEmbeddings {
    pub id: String,
    pub name: String,
    pub embeddings: Vec<Vec<f32>>,
}

pub struct NewSample {
    pub embedding: Vec<f32>,
    pub start: f64,
    pub end: f64,
}

pub fn encode_embedding(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|value| value.to_le_bytes()).collect()
}

pub fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    (bytes.len() % 4 == 0 && !bytes.is_empty()).then(|| {
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    })
}

impl VoiceprintRepository {
    pub async fn list_profiles(
        pool: &SqlitePool,
        extractor: &str,
    ) -> Result<Vec<VoiceProfileInfo>, SqlxError> {
        let profiles = sqlx::query_as::<_, (String, String, String, String, i64)>(
            "SELECT p.id, p.name, p.created_at, p.updated_at,
                    (SELECT COUNT(*) FROM meeting_speaker_profiles l
                      WHERE l.profile_id = p.id AND l.status = 'confirmed')
             FROM voice_profiles p ORDER BY p.name COLLATE NOCASE, p.created_at",
        )
        .fetch_all(pool)
        .await?;
        let samples = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, f64, f64, String, String)>(
            "SELECT s.id, s.profile_id, s.meeting_id, m.title, s.start_time, s.end_time, s.extractor, s.created_at
             FROM voice_samples s LEFT JOIN meetings m ON m.id = s.meeting_id
             ORDER BY s.created_at, s.start_time",
        )
        .fetch_all(pool)
        .await?;
        Ok(profiles
            .into_iter()
            .map(|(id, name, created_at, updated_at, confirmed_meetings)| VoiceProfileInfo {
                samples: samples
                    .iter()
                    .filter(|sample| sample.1 == id)
                    .map(|(sample_id, _, meeting_id, title, start, end, sample_extractor, created)| VoiceSampleInfo {
                        id: sample_id.clone(),
                        meeting_id: meeting_id.clone(),
                        meeting_title: title.clone(),
                        start: *start,
                        end: *end,
                        current: sample_extractor == extractor,
                        created_at: created.clone(),
                    })
                    .collect(),
                id,
                name,
                created_at,
                updated_at,
                confirmed_meetings,
            })
            .collect())
    }

    pub async fn profile_name(pool: &SqlitePool, id: &str) -> Result<Option<String>, SqlxError> {
        sqlx::query_scalar("SELECT name FROM voice_profiles WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    pub async fn profile_embeddings(
        pool: &SqlitePool,
        extractor: &str,
    ) -> Result<Vec<ProfileEmbeddings>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, String, Vec<u8>)>(
            "SELECT p.id, p.name, s.embedding FROM voice_profiles p
             JOIN voice_samples s ON s.profile_id = p.id
             WHERE s.extractor = ? ORDER BY p.id",
        )
        .bind(extractor)
        .fetch_all(pool)
        .await?;
        let mut profiles: Vec<ProfileEmbeddings> = Vec::new();
        for (id, name, blob) in rows {
            let Some(embedding) = decode_embedding(&blob) else {
                continue;
            };
            match profiles.last_mut() {
                Some(profile) if profile.id == id => profile.embeddings.push(embedding),
                _ => profiles.push(ProfileEmbeddings {
                    id,
                    name,
                    embeddings: vec![embedding],
                }),
            }
        }
        Ok(profiles)
    }

    pub async fn create_profile(pool: &SqlitePool, name: &str) -> Result<String, SqlxError> {
        let id = format!("voice-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO voice_profiles (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(name)
            .bind(&now)
            .bind(&now)
            .execute(pool)
            .await?;
        Ok(id)
    }

    /// Adds samples to a profile. A sample of the same recording interval
    /// replaces the stored one (moving it to this profile if needed), so
    /// remembering the same speaker twice never duplicates samples.
    pub async fn add_samples(
        pool: &SqlitePool,
        profile_id: &str,
        extractor: &str,
        meeting_id: &str,
        samples: &[NewSample],
    ) -> Result<(), SqlxError> {
        let now = Utc::now().to_rfc3339();
        let mut transaction = pool.begin().await?;
        for sample in samples {
            sqlx::query(
                "INSERT INTO voice_samples (id, profile_id, extractor, embedding, meeting_id, start_time, end_time, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(extractor, meeting_id, start_time, end_time)
                 DO UPDATE SET profile_id = excluded.profile_id, embedding = excluded.embedding",
            )
            .bind(format!("sample-{}", Uuid::new_v4()))
            .bind(profile_id)
            .bind(extractor)
            .bind(encode_embedding(&sample.embedding))
            .bind(meeting_id)
            .bind(sample.start)
            .bind(sample.end)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("UPDATE voice_profiles SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(profile_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await
    }

    pub async fn rename_profile(pool: &SqlitePool, id: &str, name: &str) -> Result<bool, SqlxError> {
        let result = sqlx::query("UPDATE voice_profiles SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(Utc::now().to_rfc3339())
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Forgets a voice: samples and links go, while names already confirmed
    /// in meetings stay in their transcripts.
    pub async fn delete_profile(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        let mut transaction = pool.begin().await?;
        for table in ["meeting_speaker_profiles", "voice_samples"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE profile_id = ?"))
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        let result = sqlx::query("DELETE FROM voice_profiles WHERE id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// The profile representation is computed from its samples on every
    /// match, so removing a sample is all it takes to update it.
    pub async fn delete_sample(pool: &SqlitePool, id: &str) -> Result<bool, SqlxError> {
        let result = sqlx::query("DELETE FROM voice_samples WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn meeting_links(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<SpeakerProfileLink>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, String, String, String, Option<f64>)>(
            "SELECT l.speaker, l.profile_id, p.name, l.status, l.similarity
             FROM meeting_speaker_profiles l JOIN voice_profiles p ON p.id = l.profile_id
             WHERE l.meeting_id = ? ORDER BY l.speaker",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(speaker, profile_id, profile_name, status, similarity)| SpeakerProfileLink {
                speaker,
                profile_id,
                profile_name,
                status,
                similarity,
            })
            .collect())
    }

    pub async fn link(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker: &str,
    ) -> Result<Option<SpeakerProfileLink>, SqlxError> {
        Ok(Self::meeting_links(pool, meeting_id)
            .await?
            .into_iter()
            .find(|link| link.speaker == speaker))
    }

    pub async fn set_link(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker: &str,
        profile_id: &str,
        status: &str,
        similarity: Option<f64>,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO meeting_speaker_profiles (meeting_id, speaker, profile_id, status, similarity, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(meeting_id, speaker) DO UPDATE SET
               profile_id = excluded.profile_id, status = excluded.status,
               similarity = excluded.similarity, updated_at = excluded.updated_at",
        )
        .bind(meeting_id)
        .bind(speaker)
        .bind(profile_id)
        .bind(status)
        .bind(similarity)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Stores fresh suggestions, never overwriting a confirmed or rejected
    /// decision for the same speaker.
    /// Store new suggestions. A suggestion replaces a rejection of another
    /// profile; confirmed links are never touched. `clear_pending` drops the
    /// meeting's earlier suggestions first, as after a fresh diarization.
    pub async fn replace_suggestions(
        pool: &SqlitePool,
        meeting_id: &str,
        suggestions: &[(String, String, f64)],
        clear_pending: bool,
    ) -> Result<(), SqlxError> {
        let now = Utc::now().to_rfc3339();
        let mut transaction = pool.begin().await?;
        if clear_pending {
            sqlx::query("DELETE FROM meeting_speaker_profiles WHERE meeting_id = ? AND status = 'suggested'")
                .bind(meeting_id)
                .execute(&mut *transaction)
                .await?;
        }
        for (speaker, profile_id, similarity) in suggestions {
            sqlx::query(
                "INSERT INTO meeting_speaker_profiles (meeting_id, speaker, profile_id, status, similarity, updated_at)
                 VALUES (?, ?, ?, 'suggested', ?, ?)
                 ON CONFLICT (meeting_id, speaker) DO UPDATE SET
                     profile_id = excluded.profile_id, status = excluded.status,
                     similarity = excluded.similarity, updated_at = excluded.updated_at
                 WHERE meeting_speaker_profiles.status = 'rejected'
                   AND meeting_speaker_profiles.profile_id <> excluded.profile_id",
            )
            .bind(meeting_id)
            .bind(speaker)
            .bind(profile_id)
            .bind(similarity)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await
    }

    /// Re-diarization renumbers speakers, so a suggestion or rejection made
    /// for "Speaker 2" says nothing about the new "Speaker 2". Confirmed links
    /// survive only while their label still exists: the label is the
    /// confirmed name, which `replace_speaker_turns` carries over just for
    /// turns whose source interval matches exactly.
    pub(crate) async fn reconcile_after_diarization(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        meeting_id: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM meeting_speaker_profiles WHERE meeting_id = ?
               AND (status != 'confirmed' OR speaker NOT IN
                    (SELECT speaker FROM diarization_turns WHERE meeting_id = ?))",
        )
        .bind(meeting_id)
        .bind(meeting_id)
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    /// Keeps links attached to their speaker through a rename. A manual name
    /// replaces any pending suggestion; on a merge a confirmed link wins.
    pub(crate) async fn follow_speaker_rename(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        meeting_id: &str,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "DELETE FROM meeting_speaker_profiles WHERE meeting_id = ? AND speaker = ? AND status != 'confirmed'",
        )
        .bind(meeting_id)
        .bind(old_name)
        .execute(&mut **transaction)
        .await?;
        sqlx::query(
            "DELETE FROM meeting_speaker_profiles WHERE meeting_id = ? AND speaker = ? AND status != 'confirmed'
               AND EXISTS (SELECT 1 FROM meeting_speaker_profiles WHERE meeting_id = ? AND speaker = ?)",
        )
        .bind(meeting_id)
        .bind(new_name)
        .bind(meeting_id)
        .bind(old_name)
        .execute(&mut **transaction)
        .await?;
        sqlx::query("UPDATE OR IGNORE meeting_speaker_profiles SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
            .bind(new_name)
            .bind(meeting_id)
            .bind(old_name)
            .execute(&mut **transaction)
            .await?;
        sqlx::query("DELETE FROM meeting_speaker_profiles WHERE meeting_id = ? AND speaker = ?")
            .bind(meeting_id)
            .bind(old_name)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    /// Speakers the user already named by hand: some of their turns carry a
    /// rename (`original_speaker`). These get no suggestions.
    pub async fn manually_named_speakers(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<String>, SqlxError> {
        sqlx::query_scalar(
            "SELECT DISTINCT speaker FROM diarization_turns WHERE meeting_id = ? AND original_speaker IS NOT NULL",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }
}
