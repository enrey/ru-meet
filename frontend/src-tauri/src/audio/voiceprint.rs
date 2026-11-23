//! Local voiceprint library: remember a colleague's voice from one meeting and
//! suggest their name for an anonymous speaker of the next one.
//!
//! Every engine's speaker intervals go through one fixed extractor - polyvoice's
//! native WeSpeaker ResNet34 over the `resnet34_int8` weights - so a profile
//! keeps working when the user switches between polyvoice, speakrs and
//! Sortformer. Embeddings are compared by cosine similarity only within the
//! same `EXTRACTOR` version. Nothing here touches transcripts until the user
//! confirms a suggestion, which renames the speaker like a manual rename does.

use super::diarization::{SpeakerTurn, OTHERS_SPEAKER};
use crate::database::repositories::{
    transcript::TranscriptsRepository,
    voiceprint::{NewSample, SpeakerProfileLink, VoiceProfileInfo, VoiceprintRepository},
};
use crate::database::repositories::voiceprint::{STATUS_CONFIRMED, STATUS_REJECTED, STATUS_SUGGESTED};
use crate::state::AppState;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Version of the embedding extractor and its preprocessing (80-bin fbank,
/// CMVN, L2 normalization). Change it whenever the model or the
/// preprocessing changes: older samples then stop taking part in matching.
pub const EXTRACTOR: &str = "wespeaker-resnet34-int8/polyvoice-1.0";
const EXTRACTOR_MODEL: &str = "resnet34_int8";
const SAMPLE_RATE: f64 = 16_000.0;

/// Turn boundaries are imprecise; trim this much from both ends of a piece.
const EDGE_TRIM_SECONDS: f64 = 0.15;
/// Longer clean speech is cropped around its middle to this length.
const MAX_CLIP_SECONDS: f64 = 10.0;
/// Shorter audio makes an unreliable embedding.
const MIN_EMBED_SECONDS: f64 = 1.0;

const ENROLL_MIN_CLIP_SECONDS: f64 = 2.0;
const ENROLL_MAX_CLIPS: usize = 8;
const ENROLL_MIN_CLIPS: usize = 2;
/// A short "yes" is not enough to register a person.
const ENROLL_MIN_SECONDS: f64 = 8.0;
/// Minimal similarity of a fragment to the rest of the selection.
const ENROLL_CONSISTENCY: f32 = 0.45;

const MATCH_MIN_CLIP_SECONDS: f64 = 1.5;
const MATCH_MAX_CLIPS: usize = 6;
const MATCH_MIN_CLIPS: usize = 2;
// Preliminary thresholds for WeSpeaker ResNet34 cosine similarity. On four
// speakrs-diarized meetings (`voiceprint_report` below) one voice scored
// 0.80-0.87 against its profile from another meeting, different people
// 0.0-0.2, and 0.4-0.65 was ambiguous. The primary goal is that suggested
// names are right, so these err on abstaining; retune them on more meetings
// with known and new participants.
/// Mean similarity of the speaker's fragments to the best profile.
const SUGGEST_THRESHOLD: f32 = 0.6;
/// How far the best profile must lead the second one.
const SUGGEST_MARGIN: f32 = 0.1;
/// A fragment votes for a profile only above this similarity.
const FRAGMENT_THRESHOLD: f32 = 0.35;
/// Share of fragments that must vote for the best profile.
const FRAGMENT_AGREEMENT: f64 = 2.0 / 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub start: f64,
    pub end: f64,
}

impl Clip {
    fn seconds(&self) -> f64 {
        self.end - self.start
    }
}

fn round_ms(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Intervals where only `speaker` is heard, as diarization sees it: their
/// turns minus every other speaker's (including `Others`), trimmed at the
/// edges and at least `min_seconds` long.
pub(crate) fn clean_intervals(turns: &[SpeakerTurn], speaker: &str, min_seconds: f64) -> Vec<Clip> {
    let valid = |turn: &&SpeakerTurn| turn.start.is_finite() && turn.end.is_finite() && turn.end > turn.start;
    let mut others: Vec<(f64, f64)> = turns
        .iter()
        .filter(valid)
        .filter(|turn| turn.speaker != speaker)
        .map(|turn| (turn.start, turn.end))
        .collect();
    others.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut own: Vec<(f64, f64)> = turns
        .iter()
        .filter(valid)
        .filter(|turn| turn.speaker == speaker)
        .map(|turn| (turn.start.max(0.0), turn.end))
        .collect();
    own.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Engines may split one utterance into touching turns.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in own {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }

    let mut clean = Vec::new();
    for (start, end) in merged {
        let mut cursor = start;
        for &(other_start, other_end) in &others {
            if other_end <= cursor || other_start >= end {
                continue;
            }
            if other_start > cursor {
                clean.push((cursor, other_start));
            }
            cursor = cursor.max(other_end);
            if cursor >= end {
                break;
            }
        }
        if cursor < end {
            clean.push((cursor, end));
        }
    }
    clean
        .into_iter()
        .map(|(start, end)| Clip {
            start: round_ms(start + EDGE_TRIM_SECONDS),
            end: round_ms(end - EDGE_TRIM_SECONDS),
        })
        .filter(|clip| clip.seconds() >= min_seconds)
        .collect()
}

/// The longest `max_clips` intervals, each cropped around its middle to
/// `MAX_CLIP_SECONDS`, in recording order. Deterministic, so the same
/// diarization always yields the same samples.
pub(crate) fn pick_clips(mut intervals: Vec<Clip>, max_clips: usize) -> Vec<Clip> {
    intervals.sort_by(|a, b| b.seconds().total_cmp(&a.seconds()).then(a.start.total_cmp(&b.start)));
    intervals.truncate(max_clips);
    let mut clips: Vec<Clip> = intervals
        .into_iter()
        .map(|clip| {
            if clip.seconds() <= MAX_CLIP_SECONDS {
                return clip;
            }
            let start = round_ms(clip.start + (clip.seconds() - MAX_CLIP_SECONDS) / 2.0);
            Clip {
                start,
                end: round_ms(start + MAX_CLIP_SECONDS),
            }
        })
        .collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    clips
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::NEG_INFINITY;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn normalize(mut vector: Vec<f32>) -> Option<Vec<f32>> {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return None;
    }
    vector.iter_mut().for_each(|x| *x /= norm);
    Some(vector)
}

/// Normalized mean of normalized vectors: the profile representation.
pub(crate) fn centroid<'a>(vectors: impl IntoIterator<Item = &'a Vec<f32>>) -> Option<Vec<f32>> {
    let mut sum: Option<Vec<f32>> = None;
    for vector in vectors {
        match &mut sum {
            None => sum = Some(vector.clone()),
            Some(total) if total.len() == vector.len() => {
                total.iter_mut().zip(vector).for_each(|(t, x)| *t += x)
            }
            Some(_) => {}
        }
    }
    normalize(sum?)
}

/// Indices of fragments that agree with each other. The least similar
/// fragment is dropped while it is below `ENROLL_CONSISTENCY` relative to
/// the others, so one mislabelled fragment cannot spoil a profile.
pub(crate) fn consistent_fragments(embeddings: &[Vec<f32>]) -> Vec<usize> {
    let mut kept: Vec<usize> = (0..embeddings.len()).collect();
    while kept.len() >= 2 {
        let worst = kept
            .iter()
            .map(|&index| {
                let rest = centroid(kept.iter().filter(|&&other| other != index).map(|&other| &embeddings[other]));
                (index, rest.map_or(f32::NEG_INFINITY, |rest| dot(&embeddings[index], &rest)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match worst {
            Some((index, score)) if score < ENROLL_CONSISTENCY => kept.retain(|&other| other != index),
            _ => break,
        }
    }
    if kept.len() < 2 {
        kept.clear();
    }
    kept
}

/// The profile to suggest for one speaker, with its mean similarity, or
/// `None` when the match is weak, ambiguous or rests on too few fragments.
pub(crate) fn suggest(fragments: &[Vec<f32>], profiles: &[Vec<f32>]) -> Option<(usize, f32)> {
    if fragments.len() < MATCH_MIN_CLIPS || profiles.is_empty() {
        return None;
    }
    let similarity: Vec<Vec<f32>> = fragments
        .iter()
        .map(|fragment| profiles.iter().map(|profile| dot(fragment, profile)).collect())
        .collect();
    let mean: Vec<f32> = (0..profiles.len())
        .map(|profile| similarity.iter().map(|row| row[profile]).sum::<f32>() / fragments.len() as f32)
        .collect();
    let mut ranked: Vec<usize> = (0..profiles.len()).collect();
    ranked.sort_by(|&a, &b| mean[b].total_cmp(&mean[a]));
    let best = ranked[0];
    // A single profile still has to clear the absolute threshold.
    if !(mean[best] >= SUGGEST_THRESHOLD) {
        return None;
    }
    if ranked.get(1).is_some_and(|&second| mean[best] - mean[second] < SUGGEST_MARGIN) {
        return None;
    }
    let votes = similarity
        .iter()
        .filter(|row| {
            row[best] >= FRAGMENT_THRESHOLD
                && row.iter().enumerate().all(|(profile, value)| profile == best || *value < row[best])
        })
        .count();
    (votes >= MATCH_MIN_CLIPS && votes as f64 >= FRAGMENT_AGREEMENT * fragments.len() as f64)
        .then_some((best, mean[best]))
}

/// Worker side: embed recording intervals with the fixed extractor. `None`
/// marks an interval too short (or outside the recording) to embed.
pub(crate) fn embed_in_worker(path: &std::path::Path, clips: &[(f64, f64)]) -> Result<Vec<Option<Vec<f32>>>> {
    let samples = super::decoder::decode_audio_file(path)?.to_whisper_format();
    embed_samples(&samples, clips)
}

/// Embed intervals of already decoded 16 kHz mono samples, L2-normalized.
pub(crate) fn embed_samples(samples: &[f32], clips: &[(f64, f64)]) -> Result<Vec<Option<Vec<f32>>>> {
    use polyvoice::Embedder;
    let registry = polyvoice::ModelRegistry::with_cache_dir(super::diarization::models_root()?)
        .map_err(|error| anyhow!("Could not open the voice model cache: {error}"))?;
    let model = registry
        .ensure(EXTRACTOR_MODEL)
        .map_err(|error| anyhow!("Could not load the voice model: {error}"))?;
    let embedder = polyvoice::ResNet34Native::from_onnx_path(model)?;
    clips
        .iter()
        .map(|&(start, end)| {
            let from = ((start.max(0.0) * SAMPLE_RATE) as usize).min(samples.len());
            let to = ((end.max(0.0) * SAMPLE_RATE) as usize).min(samples.len());
            if to <= from || ((to - from) as f64) < MIN_EMBED_SECONDS * SAMPLE_RATE {
                return Ok(None);
            }
            Ok(normalize(embedder.embed(&samples[from..to])?))
        })
        .collect()
}

async fn meeting_audio(pool: &SqlitePool, meeting_id: &str) -> Result<PathBuf, String> {
    let folder: Option<String> = sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?
        .flatten();
    folder
        .and_then(|folder| super::retranscription::find_audio_file(std::path::Path::new(&folder)).ok())
        .ok_or_else(|| "This meeting has no saved recording".to_string())
}

/// Every label used in a meeting, by the timeline or the transcript.
async fn speaker_labels(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<String>, String> {
    sqlx::query_scalar(
        "SELECT speaker FROM diarization_turns WHERE meeting_id = ?
         UNION SELECT speaker FROM transcripts WHERE meeting_id = ? AND speaker IS NOT NULL",
    )
    .bind(meeting_id)
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())
}

/// Whether `name` may become the label of `speaker`: it is free, or belongs
/// to a speaker already confirmed as the same person (renaming then merges).
async fn name_is_free(
    pool: &SqlitePool,
    meeting_id: &str,
    speaker: &str,
    name: &str,
    profile_id: &str,
) -> Result<bool, String> {
    if speaker == name || !speaker_labels(pool, meeting_id).await?.iter().any(|label| label == name) {
        return Ok(true);
    }
    let owner = VoiceprintRepository::link(pool, meeting_id, name)
        .await
        .map_err(|error| error.to_string())?;
    Ok(owner.is_some_and(|link| link.status == STATUS_CONFIRMED && link.profile_id == profile_id))
}

async fn rename_confirmed<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    speaker: &str,
    name: &str,
) -> Result<(), String> {
    if speaker != name {
        TranscriptsRepository::rename_speaker(pool, meeting_id, speaker, name).await?;
    }
    super::transcript_export::export_diarized_transcripts_logged(pool, meeting_id).await;
    let _ = app.emit("transcript-speakers-updated", serde_json::json!({ "meetingId": meeting_id }));
    Ok(())
}

fn emit_links_changed<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    let _ = app.emit("voice-suggestions-updated", serde_json::json!({ "meetingId": meeting_id }));
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentCandidates {
    pub clips: Vec<Clip>,
    pub total_seconds: f64,
    pub min_clips: usize,
    pub min_seconds: f64,
    /// Why the speaker cannot be remembered from this meeting: `others`,
    /// `no-audio` or `too-little-speech`.
    pub problem: Option<String>,
}

async fn enrollment_clips(pool: &SqlitePool, meeting_id: &str, speaker: &str) -> Result<Vec<Clip>, String> {
    let turns = TranscriptsRepository::get_speaker_turns(pool, meeting_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(pick_clips(clean_intervals(&turns, speaker, ENROLL_MIN_CLIP_SECONDS), ENROLL_MAX_CLIPS))
}

#[tauri::command]
pub async fn get_voice_enrollment_candidates(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    speaker: String,
) -> Result<EnrollmentCandidates, String> {
    let pool = state.db_manager.pool();
    let clips = if speaker == OTHERS_SPEAKER {
        Vec::new()
    } else {
        enrollment_clips(pool, &meeting_id, &speaker).await?
    };
    let total_seconds: f64 = clips.iter().map(Clip::seconds).sum();
    let problem = if speaker == OTHERS_SPEAKER {
        Some("others")
    } else if meeting_audio(pool, &meeting_id).await.is_err() {
        Some("no-audio")
    } else if clips.len() < ENROLL_MIN_CLIPS || total_seconds < ENROLL_MIN_SECONDS {
        Some("too-little-speech")
    } else {
        None
    };
    Ok(EnrollmentCandidates {
        clips,
        total_seconds,
        min_clips: ENROLL_MIN_CLIPS,
        min_seconds: ENROLL_MIN_SECONDS,
        problem: problem.map(str::to_owned),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RememberVoiceResult {
    pub profile_id: String,
    pub name: String,
    pub kept: usize,
    pub discarded: usize,
    /// The speaker's label after remembering; unchanged when the profile
    /// name is already taken by another speaker of this meeting.
    pub speaker: String,
}

fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("Speaker name must be between 1 and 80 characters".into());
    }
    Ok(name.to_owned())
}

/// Remember the voice of `speaker` from the chosen fragments, into a new
/// profile (`name`) or an existing one (`profile_id`).
#[tauri::command]
pub async fn remember_voice<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    speaker: String,
    clips: Vec<Clip>,
    profile_id: Option<String>,
    name: Option<String>,
) -> Result<RememberVoiceResult, String> {
    let pool = state.db_manager.pool();
    if speaker == OTHERS_SPEAKER {
        return Err("Voices of the Others group are mixed and cannot be remembered".into());
    }
    let existing = match &profile_id {
        Some(id) => Some(
            VoiceprintRepository::profile_name(pool, id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or("This voice profile no longer exists")?,
        ),
        None => None,
    };
    let name = match existing {
        Some(name) => name,
        None => validate_name(name.as_deref().unwrap_or_default())?,
    };

    // Only fragments the app itself offered: single-speaker, not `Others`.
    let candidates = enrollment_clips(pool, &meeting_id, &speaker).await?;
    if clips.iter().any(|clip| !candidates.iter().any(|candidate| {
        (candidate.start - clip.start).abs() < 1e-6 && (candidate.end - clip.end).abs() < 1e-6
    })) {
        return Err("The fragments no longer match the speaker timeline. Open the dialog again.".into());
    }
    let too_little = |clips: &[Clip]| {
        clips.len() < ENROLL_MIN_CLIPS || clips.iter().map(Clip::seconds).sum::<f64>() < ENROLL_MIN_SECONDS
    };
    if too_little(&clips) {
        return Err("Too little speech selected: choose at least 2 fragments with 8 seconds of speech in total".into());
    }

    let requested = clips.len();
    let audio = meeting_audio(pool, &meeting_id).await?;
    let embeddings = crate::background_worker::voiceprints(
        &app,
        audio,
        clips.iter().map(|clip| (clip.start, clip.end)).collect(),
    )
    .await
    .map_err(|error| format!("Could not analyse the voice: {error:#}"))?;
    let (clips, embeddings): (Vec<Clip>, Vec<Vec<f32>>) = clips
        .into_iter()
        .zip(embeddings)
        .filter_map(|(clip, embedding)| embedding.map(|embedding| (clip, embedding)))
        .unzip();
    let kept = consistent_fragments(&embeddings);
    let kept_clips: Vec<Clip> = kept.iter().map(|&index| clips[index]).collect();
    if too_little(&kept_clips) {
        return Err("The selected fragments sound like different people. Exclude mislabelled fragments or choose another recording.".into());
    }

    let profile_id = match profile_id {
        Some(id) => id,
        None => VoiceprintRepository::create_profile(pool, &name)
            .await
            .map_err(|error| error.to_string())?,
    };
    let samples: Vec<NewSample> = kept
        .iter()
        .map(|&index| NewSample {
            embedding: embeddings[index].clone(),
            start: clips[index].start,
            end: clips[index].end,
        })
        .collect();
    VoiceprintRepository::add_samples(pool, &profile_id, EXTRACTOR, &meeting_id, &samples)
        .await
        .map_err(|error| error.to_string())?;
    VoiceprintRepository::set_link(pool, &meeting_id, &speaker, &profile_id, STATUS_CONFIRMED, None)
        .await
        .map_err(|error| error.to_string())?;
    let label = if name_is_free(pool, &meeting_id, &speaker, &name, &profile_id).await? {
        rename_confirmed(&app, pool, &meeting_id, &speaker, &name).await?;
        name.clone()
    } else {
        speaker.clone()
    };
    emit_links_changed(&app, &meeting_id);
    let _ = app.emit("voice-library-updated", ());
    queue_library_matching(app.clone());
    Ok(RememberVoiceResult {
        profile_id,
        name,
        kept: samples.len(),
        discarded: requested - samples.len(),
        speaker: label,
    })
}

#[tauri::command]
pub async fn get_meeting_voice_links(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<SpeakerProfileLink>, String> {
    VoiceprintRepository::meeting_links(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| error.to_string())
}

/// Accept "Sounds like Anna": the speaker is renamed to the profile name in
/// the timeline, the transcript and the exports. Adds no samples.
#[tauri::command]
pub async fn confirm_voice_suggestion<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    speaker: String,
) -> Result<String, String> {
    let pool = state.db_manager.pool();
    let link = VoiceprintRepository::link(pool, &meeting_id, &speaker)
        .await
        .map_err(|error| error.to_string())?
        .filter(|link| link.status == STATUS_SUGGESTED)
        .ok_or("This suggestion is no longer current")?;
    let name = link.profile_name;
    if !name_is_free(pool, &meeting_id, &speaker, &name, &link.profile_id).await? {
        return Err(format!(
            "Another speaker in this meeting is already named “{name}”. Rename that speaker first."
        ));
    }
    VoiceprintRepository::set_link(pool, &meeting_id, &speaker, &link.profile_id, STATUS_CONFIRMED, link.similarity)
        .await
        .map_err(|error| error.to_string())?;
    rename_confirmed(&app, pool, &meeting_id, &speaker, &name).await?;
    emit_links_changed(&app, &meeting_id);
    Ok(name)
}

/// Decline a suggestion: the label stays and no sample is added.
#[tauri::command]
pub async fn reject_voice_suggestion<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    speaker: String,
) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let link = VoiceprintRepository::link(pool, &meeting_id, &speaker)
        .await
        .map_err(|error| error.to_string())?
        .filter(|link| link.status == STATUS_SUGGESTED)
        .ok_or("This suggestion is no longer current")?;
    VoiceprintRepository::set_link(pool, &meeting_id, &speaker, &link.profile_id, STATUS_REJECTED, link.similarity)
        .await
        .map_err(|error| error.to_string())?;
    emit_links_changed(&app, &meeting_id);
    Ok(())
}

#[tauri::command]
pub async fn list_voice_profiles(state: tauri::State<'_, AppState>) -> Result<Vec<VoiceProfileInfo>, String> {
    VoiceprintRepository::list_profiles(state.db_manager.pool(), EXTRACTOR)
        .await
        .map_err(|error| error.to_string())
}

/// Renames the profile only: names already confirmed in meetings stay as
/// they are in their history.
#[tauri::command]
pub async fn rename_voice_profile<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    profile_id: String,
    name: String,
) -> Result<(), String> {
    let name = validate_name(&name)?;
    if !VoiceprintRepository::rename_profile(state.db_manager.pool(), &profile_id, &name)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("This voice profile no longer exists".into());
    }
    let _ = app.emit("voice-library-updated", ());
    Ok(())
}

#[tauri::command]
pub async fn delete_voice_profile<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    VoiceprintRepository::delete_profile(state.db_manager.pool(), &profile_id)
        .await
        .map_err(|error| error.to_string())?;
    let _ = app.emit("voice-library-updated", ());
    let _ = app.emit("voice-suggestions-updated", serde_json::json!({}));
    // Without this voice another profile may now lead clearly enough.
    queue_library_matching(app);
    Ok(())
}

#[tauri::command]
pub async fn delete_voice_sample<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    sample_id: String,
) -> Result<(), String> {
    VoiceprintRepository::delete_sample(state.db_manager.pool(), &sample_id)
        .await
        .map_err(|error| error.to_string())?;
    let _ = app.emit("voice-library-updated", ());
    queue_library_matching(app);
    Ok(())
}

/// Look for familiar voices among the meeting's anonymous speakers again,
/// e.g. after the library grew. Returns the number of suggestions.
#[tauri::command]
pub async fn refresh_voice_suggestions<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<usize, String> {
    let pool = state.db_manager.pool().clone();
    let count = compute_suggestions(&app, &pool, &meeting_id, Recheck::All)
        .await
        .map_err(|error| format!("{error:#}"))?;
    emit_links_changed(&app, &meeting_id);
    Ok(count)
}

/// Queue a search for the library's people in one meeting. Returns `false`
/// when one already waits in the queue: it has not started, so it will see
/// the library as it is now.
pub(crate) fn queue_meeting_matching<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> bool {
    use crate::tasks::{Action, Task};
    let action = Action::VoiceMatch { meeting_id: meeting_id.clone() };
    if crate::tasks::is_waiting(&action) {
        return false;
    }
    let task = Task::register(action);
    tauri::async_runtime::spawn(async move {
        task.run(async {
            let Some(state) = app.try_state::<AppState>() else {
                crate::tasks::fail_current("The database is unavailable");
                return;
            };
            let pool = state.db_manager.pool().clone();
            match compute_suggestions(&app, &pool, &meeting_id, Recheck::KeepPending).await {
                Ok(count) => {
                    if count > 0 {
                        log::info!("Voiceprints: {count} speaker name suggestions for meeting {meeting_id}");
                    }
                    emit_links_changed(&app, &meeting_id);
                }
                Err(error) => {
                    log::warn!("Voiceprints: could not match meeting {meeting_id}: {error:#}");
                    crate::tasks::fail_current(format!("{error:#}"));
                }
            }
        })
        .await;
    });
    true
}

/// Meetings, newest first, with a speaker the library could still name: not
/// `Others`, not named by hand, not confirmed and without a pending
/// suggestion. A rejected suggestion may be followed by another person.
async fn meetings_to_match(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT d.meeting_id FROM diarization_turns d
         JOIN meetings m ON m.id = d.meeting_id
         WHERE d.speaker <> ? AND d.original_speaker IS NULL
           AND NOT EXISTS (SELECT 1 FROM diarization_turns n
               WHERE n.meeting_id = d.meeting_id AND n.speaker = d.speaker AND n.original_speaker IS NOT NULL)
           AND NOT EXISTS (SELECT 1 FROM meeting_speaker_profiles l
               WHERE l.meeting_id = d.meeting_id AND l.speaker = d.speaker AND l.status <> ?)
         GROUP BY d.meeting_id
         ORDER BY MAX(m.created_at) DESC",
    )
    .bind(OTHERS_SPEAKER)
    .bind(STATUS_REJECTED)
    .fetch_all(pool)
    .await
}

/// One matching task per meeting that has a recording and someone to name.
async fn queue_all_meetings<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> Result<usize, String> {
    let mut queued = 0;
    for meeting_id in meetings_to_match(pool).await.map_err(|error| error.to_string())? {
        if meeting_audio(pool, &meeting_id).await.is_ok() && queue_meeting_matching(app.clone(), meeting_id) {
            queued += 1;
        }
    }
    Ok(queued)
}

/// After the library changed: look for its people in every meeting again.
pub(crate) fn queue_library_matching<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<AppState>() else { return };
        let pool = state.db_manager.pool().clone();
        match queue_all_meetings(&app, &pool).await {
            Ok(queued) => log::info!("Voiceprints: library changed, {queued} meetings queued for matching"),
            Err(error) => log::warn!("Voiceprints: could not queue library matching: {error}"),
        }
    });
}

/// Look for the library's people in all meetings, from the meetings list.
/// Returns the number of meetings queued.
#[tauri::command]
pub async fn match_voice_library<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    let pool = state.db_manager.pool();
    let profiles = VoiceprintRepository::profile_embeddings(pool, EXTRACTOR)
        .await
        .map_err(|error| error.to_string())?;
    if profiles.iter().all(|profile| profile.embeddings.is_empty()) {
        return Err("The voice library is empty. Remember a voice in a meeting first.".into());
    }
    queue_all_meetings(&app, pool).await
}

type ClipKey = (u64, u64);
/// Fragment embeddings by meeting. The recording does not change when the
/// library does, so a pass after adding a voice decodes no audio again.
static CLIP_EMBEDDINGS: Lazy<std::sync::Mutex<HashMap<String, (PathBuf, HashMap<ClipKey, Option<Vec<f32>>>)>>> =
    Lazy::new(Default::default);
/// About 6 KB per speaker; beyond this many meetings the cache starts over.
const CACHED_MEETINGS: usize = 300;

async fn clip_embeddings<R: Runtime>(
    app: &AppHandle<R>,
    meeting_id: &str,
    audio: PathBuf,
    clips: Vec<(f64, f64)>,
) -> Result<Vec<Option<Vec<f32>>>> {
    let key = |(start, end): (f64, f64)| (start.to_bits(), end.to_bits());
    let missing: Vec<(f64, f64)> = {
        let cache = CLIP_EMBEDDINGS.lock().map_err(|_| anyhow!("Voiceprint cache is unavailable"))?;
        let known = cache.get(meeting_id).filter(|(path, _)| *path == audio);
        clips
            .iter()
            .copied()
            .filter(|clip| !known.is_some_and(|(_, known)| known.contains_key(&key(*clip))))
            .collect()
    };
    let computed = if missing.is_empty() {
        Vec::new()
    } else {
        crate::background_worker::voiceprints(app, audio.clone(), missing.clone()).await?
    };
    let mut cache = CLIP_EMBEDDINGS.lock().map_err(|_| anyhow!("Voiceprint cache is unavailable"))?;
    if cache.len() >= CACHED_MEETINGS && !cache.contains_key(meeting_id) {
        cache.clear();
    }
    let entry = cache.entry(meeting_id.to_owned()).or_insert_with(|| (audio.clone(), HashMap::new()));
    if entry.0 != audio {
        *entry = (audio, HashMap::new());
    }
    for (clip, embedding) in missing.into_iter().zip(computed) {
        entry.1.insert(key(clip), embedding);
    }
    Ok(clips.into_iter().map(|clip| entry.1.get(&key(clip)).cloned().flatten()).collect())
}

/// Called once a meeting's speaker turns are stored.
pub fn spawn_suggestions<R: Runtime>(app: AppHandle<R>, pool: SqlitePool, meeting_id: String) {
    tauri::async_runtime::spawn(async move {
        match compute_suggestions(&app, &pool, &meeting_id, Recheck::All).await {
            Ok(count) => {
                if count > 0 {
                    log::info!("Voiceprints: {count} speaker name suggestions for meeting {meeting_id}");
                }
                emit_links_changed(&app, &meeting_id);
            }
            Err(error) => log::warn!("Voiceprints: could not suggest names for meeting {meeting_id}: {error:#}"),
        }
    });
}

static MEETING_LOCKS: Lazy<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Lazy::new(Default::default);

/// Which speakers a search looks at again.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Recheck {
    /// Every speaker without a confirmed or manual name: after diarization,
    /// or when the user asks for one meeting.
    All,
    /// Pending suggestions stay as they are; only speakers without one, or
    /// whose suggestion was rejected, are looked at. After a library change
    /// this keeps passes over a large archive from growing with every
    /// suggestion nobody got around to confirming.
    KeepPending,
}

/// One person is one speaker of a meeting: a profile already linked to a
/// speaker that keeps its link (`occupied`) is not suggested again, and of
/// several speakers that sound like the same profile only the most similar
/// one gets the suggestion.
fn one_speaker_per_profile<'a>(
    mut suggestions: Vec<(String, String, f64)>,
    occupied: impl IntoIterator<Item = &'a str>,
) -> Vec<(String, String, f64)> {
    let occupied: std::collections::HashSet<&str> = occupied.into_iter().collect();
    suggestions.retain(|(_, profile, _)| !occupied.contains(profile.as_str()));
    suggestions.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut taken = std::collections::HashSet::new();
    suggestions.retain(|(_, profile, _)| taken.insert(profile.clone()));
    suggestions
}

async fn compute_suggestions<R: Runtime>(
    app: &AppHandle<R>,
    pool: &SqlitePool,
    meeting_id: &str,
    recheck: Recheck,
) -> Result<usize> {
    let lock = MEETING_LOCKS
        .lock()
        .map_err(|_| anyhow!("Voiceprint lock is unavailable"))?
        .entry(meeting_id.to_owned())
        .or_default()
        .clone();
    let _guard = lock.lock().await;

    let profiles = VoiceprintRepository::profile_embeddings(pool, EXTRACTOR).await?;
    let profiles: Vec<(String, Vec<f32>)> = profiles
        .into_iter()
        .filter_map(|profile| centroid(&profile.embeddings).map(|centroid| (profile.id, centroid)))
        .collect();
    let turns = TranscriptsRepository::get_speaker_turns(pool, meeting_id).await?;
    let links = VoiceprintRepository::meeting_links(pool, meeting_id).await?;
    let manual = VoiceprintRepository::manually_named_speakers(pool, meeting_id).await?;
    let mut speakers: Vec<&str> = turns.iter().map(|turn| turn.speaker.as_str()).collect();
    speakers.sort_unstable();
    speakers.dedup();
    let link_of = |speaker: &str| links.iter().find(|link| link.speaker == speaker);
    // Links left as they are: confirmed ones, and pending ones unless rechecked.
    let kept = |link: &SpeakerProfileLink| {
        link.status == STATUS_CONFIRMED || (link.status == STATUS_SUGGESTED && recheck == Recheck::KeepPending)
    };
    let wanted: Vec<(&str, Vec<Clip>)> = speakers
        .into_iter()
        .filter(|speaker| *speaker != OTHERS_SPEAKER && !speaker.trim().is_empty())
        .filter(|speaker| !manual.iter().any(|name| name == speaker))
        .filter(|speaker| !link_of(speaker).is_some_and(kept))
        .map(|speaker| (speaker, pick_clips(clean_intervals(&turns, speaker, MATCH_MIN_CLIP_SECONDS), MATCH_MAX_CLIPS)))
        .filter(|(_, clips)| clips.len() >= MATCH_MIN_CLIPS)
        .collect();

    let mut suggestions = Vec::new();
    if !profiles.is_empty() && !wanted.is_empty() {
        let audio = meeting_audio(pool, meeting_id).await.map_err(|error| anyhow!(error))?;
        let flat: Vec<(f64, f64)> = wanted
            .iter()
            .flat_map(|(_, clips)| clips.iter().map(|clip| (clip.start, clip.end)))
            .collect();
        let mut embeddings = clip_embeddings(app, meeting_id, audio, flat).await?.into_iter();
        for (speaker, clips) in &wanted {
            let fragments: Vec<Vec<f32>> = embeddings.by_ref().take(clips.len()).flatten().collect();
            // The person the user said this is not stays out of the race.
            let rejected = link_of(speaker)
                .filter(|link| link.status == STATUS_REJECTED)
                .map(|link| link.profile_id.as_str());
            let candidates: Vec<&(String, Vec<f32>)> =
                profiles.iter().filter(|(id, _)| Some(id.as_str()) != rejected).collect();
            let centroids: Vec<Vec<f32>> = candidates.iter().map(|(_, centroid)| centroid.clone()).collect();
            if let Some((profile, similarity)) = suggest(&fragments, &centroids) {
                suggestions.push((speaker.to_string(), candidates[profile].0.clone(), similarity as f64));
            }
        }
    }
    let suggestions = one_speaker_per_profile(
        suggestions,
        links.iter().filter(|link| kept(link)).map(|link| link.profile_id.as_str()),
    );

    // Diarization may have rerun while the worker was busy; then these
    // labels mean other people and the newer run computes its own.
    if TranscriptsRepository::get_speaker_turns(pool, meeting_id)
        .await?
        .iter()
        .map(|turn| (turn.start, turn.end, &turn.speaker))
        .ne(turns.iter().map(|turn| (turn.start, turn.end, &turn.speaker)))
    {
        return Ok(0);
    }
    VoiceprintRepository::replace_suggestions(pool, meeting_id, &suggestions, recheck == Recheck::All).await?;
    Ok(suggestions.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start: f64, end: f64, speaker: &str) -> SpeakerTurn {
        SpeakerTurn { start, end, speaker: speaker.into() }
    }

    fn unit(values: &[f32]) -> Vec<f32> {
        normalize(values.to_vec()).unwrap()
    }

    #[test]
    fn clean_intervals_exclude_overlaps_and_others() {
        let turns = [
            turn(0.0, 10.0, "Speaker 1"),
            turn(4.0, 5.0, "Speaker 2"),
            turn(10.0, 12.0, "Speaker 1"),
            turn(20.0, 21.0, "Speaker 1"),
            turn(30.0, 40.0, "Speaker 1"),
            turn(31.0, 39.5, "Others"),
        ];
        let clips = clean_intervals(&turns, "Speaker 1", 2.0);
        assert_eq!(
            clips,
            [Clip { start: 0.15, end: 3.85 }, Clip { start: 5.15, end: 11.85 }],
            "the overlap splits a turn, touching turns join, short ones drop"
        );
        assert!(clean_intervals(&turns, "Speaker 3", 0.5).is_empty());
    }

    #[test]
    fn picked_clips_are_longest_cropped_and_in_order() {
        let clips = pick_clips(
            vec![
                Clip { start: 0.0, end: 3.0 },
                Clip { start: 10.0, end: 40.0 },
                Clip { start: 50.0, end: 52.0 },
                Clip { start: 60.0, end: 64.0 },
            ],
            3,
        );
        assert_eq!(
            clips,
            [Clip { start: 0.0, end: 3.0 }, Clip { start: 20.0, end: 30.0 }, Clip { start: 60.0, end: 64.0 }]
        );
    }

    #[test]
    fn suggestion_needs_threshold_margin_and_agreement() {
        let anna = unit(&[1.0, 0.0, 0.0]);
        let igor = unit(&[0.0, 1.0, 0.0]);
        let near_anna = unit(&[0.9, 0.1, 0.2]);
        let stranger = unit(&[0.1, 0.1, 1.0]);

        assert_eq!(suggest(&[near_anna.clone(), near_anna.clone()], &[anna.clone(), igor.clone()]).map(|s| s.0), Some(0));
        // A new colleague is left anonymous, even with a one-profile library.
        assert_eq!(suggest(&[stranger.clone(), stranger.clone()], &[anna.clone()]), None);
        // One fragment is not enough.
        assert_eq!(suggest(&[near_anna.clone()], &[anna.clone()]), None);
        // Two similar profiles: abstain.
        let anna_twin = unit(&[0.98, 0.0, 0.2]);
        assert_eq!(suggest(&[near_anna.clone(), near_anna.clone()], &[anna.clone(), anna_twin]), None);
        // Fragments that disagree (overlapping voices) abstain.
        let mixed = [near_anna.clone(), stranger.clone(), stranger];
        assert_eq!(suggest(&mixed, &[anna]), None);
    }

    #[test]
    fn a_profile_is_suggested_for_one_speaker_only() {
        let suggestion = |speaker: &str, profile: &str, similarity: f64| {
            (speaker.to_owned(), profile.to_owned(), similarity)
        };
        // "andrey" already belongs to a speaker of this meeting.
        let kept = one_speaker_per_profile(
            vec![
                suggestion("Speaker 2", "anna", 0.7),
                suggestion("Speaker 3", "anna", 0.8),
                suggestion("Speaker 4", "andrey", 0.9),
                suggestion("Speaker 5", "boris", 0.65),
            ],
            ["andrey"],
        );
        assert_eq!(kept, vec![suggestion("Speaker 3", "anna", 0.8), suggestion("Speaker 5", "boris", 0.65)]);
    }

    #[test]
    fn inconsistent_fragment_is_dropped() {
        let a = unit(&[1.0, 0.1, 0.0]);
        let b = unit(&[0.95, 0.0, 0.1]);
        let c = unit(&[1.0, 0.05, 0.05]);
        let other = unit(&[0.0, 1.0, 0.0]);
        assert_eq!(consistent_fragments(&[a.clone(), other.clone(), b, c]), [0, 2, 3]);
        assert!(consistent_fragments(&[a, other]).is_empty());
    }

    #[test]
    fn embeddings_round_trip_through_blobs() {
        use crate::database::repositories::voiceprint::{decode_embedding, encode_embedding};
        let vector = vec![0.25_f32, -1.5, 3.0];
        assert_eq!(decode_embedding(&encode_embedding(&vector)), Some(vector));
        assert_eq!(decode_embedding(&[1, 2, 3]), None);
    }

    /// Similarity report on real meetings, to tune the thresholds:
    /// `VOICEPRINT_DB=<copy of meeting_minutes.sqlite> VOICEPRINT_MODELS=<models/diarization>
    /// VOICEPRINT_MEETINGS=<id>,<id> cargo test -p meetily --lib voiceprint_report -- --ignored --nocapture`.
    /// Each meeting's speakers become profiles; every other meeting is
    /// matched against them.
    #[tokio::test]
    #[ignore = "needs a local database copy, recordings and the WeSpeaker model"]
    async fn voiceprint_report() {
        super::super::diarization::configure_worker(std::env::var("VOICEPRINT_MODELS").unwrap().into());
        let pool = SqlitePool::connect(&format!("sqlite:{}?mode=ro", std::env::var("VOICEPRINT_DB").unwrap()))
            .await
            .unwrap();
        let mut meetings = Vec::new();
        for meeting in std::env::var("VOICEPRINT_MEETINGS").unwrap().split(',') {
            let turns = TranscriptsRepository::get_speaker_turns(&pool, meeting).await.unwrap();
            let audio = meeting_audio(&pool, meeting).await.unwrap();
            let mut speakers: Vec<String> = turns.iter().map(|t| t.speaker.clone()).collect();
            speakers.sort();
            speakers.dedup();
            let mut voices = Vec::new();
            for speaker in speakers.into_iter().filter(|s| s != OTHERS_SPEAKER) {
                let clips = pick_clips(clean_intervals(&turns, &speaker, MATCH_MIN_CLIP_SECONDS), ENROLL_MAX_CLIPS);
                let flat: Vec<_> = clips.iter().map(|c| (c.start, c.end)).collect();
                let embeddings: Vec<Vec<f32>> = embed_in_worker(&audio, &flat).unwrap().into_iter().flatten().collect();
                let kept = consistent_fragments(&embeddings);
                println!("{meeting} {speaker}: {} clips, {} consistent", embeddings.len(), kept.len());
                voices.push((speaker, embeddings, kept));
            }
            meetings.push((meeting.to_owned(), voices));
        }
        for (enrolled_meeting, enrolled) in &meetings {
            let profiles: Vec<Vec<f32>> = enrolled
                .iter()
                .filter_map(|(_, embeddings, kept)| centroid(kept.iter().map(|&i| &embeddings[i])))
                .collect();
            let names: Vec<&str> = enrolled
                .iter()
                .filter(|(_, embeddings, kept)| centroid(kept.iter().map(|&i| &embeddings[i])).is_some())
                .map(|(speaker, _, _)| speaker.as_str())
                .collect();
            for (meeting, voices) in &meetings {
                for (speaker, embeddings, _) in voices {
                    let fragments: Vec<Vec<f32>> = embeddings.iter().take(MATCH_MAX_CLIPS).cloned().collect();
                    let means: Vec<String> = profiles
                        .iter()
                        .zip(&names)
                        .map(|(p, n)| format!("{n}={:.2}", fragments.iter().map(|f| dot(f, p)).sum::<f32>() / fragments.len().max(1) as f32))
                        .collect();
                    println!(
                        "profiles of {enrolled_meeting} vs {meeting} {speaker}: {} -> {:?}",
                        means.join(" "),
                        suggest(&fragments, &profiles).map(|(i, s)| (names[i], s))
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn only_meetings_with_someone_to_name_are_matched() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let open = TranscriptsRepository::save_transcript(&pool, "open", &[], None).await.unwrap();
        let named = TranscriptsRepository::save_transcript(&pool, "named", &[], None).await.unwrap();
        let turns = [turn(0.0, 10.0, "Speaker 1"), turn(10.0, 20.0, "Speaker 2"), turn(20.0, 30.0, OTHERS_SPEAKER)];
        TranscriptsRepository::apply_speaker_turns(&pool, &open, &turns).await.unwrap();
        TranscriptsRepository::apply_speaker_turns(&pool, &named, &[turn(0.0, 10.0, "Speaker 1"), turn(10.0, 12.0, OTHERS_SPEAKER)])
            .await
            .unwrap();
        TranscriptsRepository::rename_speaker(&pool, &named, "Speaker 1", "Игорь").await.unwrap();
        assert_eq!(meetings_to_match(&pool).await.unwrap(), [open.clone()]);

        // A pending suggestion waits for the user and is not searched again;
        // after a rejection another person may still be found.
        let anna = VoiceprintRepository::create_profile(&pool, "Анна").await.unwrap();
        let boris = VoiceprintRepository::create_profile(&pool, "Борис").await.unwrap();
        VoiceprintRepository::replace_suggestions(
            &pool,
            &open,
            &[("Speaker 1".into(), anna.clone(), 0.7), ("Speaker 2".into(), boris.clone(), 0.7)],
            true,
        )
        .await
        .unwrap();
        assert!(meetings_to_match(&pool).await.unwrap().is_empty());
        VoiceprintRepository::set_link(&pool, &open, "Speaker 2", &boris, STATUS_REJECTED, None).await.unwrap();
        assert_eq!(meetings_to_match(&pool).await.unwrap(), [open.clone()]);

        // A library pass keeps pending suggestions, replaces the rejection
        // with another person, and never brings the rejected one back.
        VoiceprintRepository::replace_suggestions(
            &pool,
            &open,
            &[("Speaker 1".into(), boris.clone(), 0.9), ("Speaker 2".into(), boris.clone(), 0.9)],
            false,
        )
        .await
        .unwrap();
        let status = |links: &[SpeakerProfileLink], speaker: &str| {
            links.iter().find(|l| l.speaker == speaker).map(|l| (l.status.clone(), l.profile_id.clone()))
        };
        let links = VoiceprintRepository::meeting_links(&pool, &open).await.unwrap();
        assert_eq!(status(&links, "Speaker 1"), Some((STATUS_SUGGESTED.into(), anna.clone())));
        assert_eq!(status(&links, "Speaker 2"), Some((STATUS_REJECTED.into(), boris.clone())));
        VoiceprintRepository::replace_suggestions(&pool, &open, &[("Speaker 2".into(), anna.clone(), 0.8)], false)
            .await
            .unwrap();
        let links = VoiceprintRepository::meeting_links(&pool, &open).await.unwrap();
        assert_eq!(status(&links, "Speaker 2"), Some((STATUS_SUGGESTED.into(), anna.clone())));
        assert!(meetings_to_match(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn links_follow_renames_and_rediarization() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let meeting = TranscriptsRepository::save_transcript(&pool, "voices", &[], None).await.unwrap();
        let turns = [turn(0.0, 10.0, "Speaker 1"), turn(10.0, 20.0, "Speaker 2"), turn(20.0, 30.0, "Speaker 3")];
        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &turns).await.unwrap();
        let anna = VoiceprintRepository::create_profile(&pool, "Анна").await.unwrap();
        let other_anna = VoiceprintRepository::create_profile(&pool, "Анна").await.unwrap();
        VoiceprintRepository::add_samples(&pool, &anna, EXTRACTOR, &meeting, &[NewSample { embedding: vec![1.0, 0.0], start: 0.0, end: 5.0 }])
            .await
            .unwrap();
        // Remembering the same interval again replaces, never duplicates.
        VoiceprintRepository::add_samples(&pool, &anna, EXTRACTOR, &meeting, &[NewSample { embedding: vec![0.0, 1.0], start: 0.0, end: 5.0 }])
            .await
            .unwrap();
        let profiles = VoiceprintRepository::list_profiles(&pool, EXTRACTOR).await.unwrap();
        assert_eq!(profiles.iter().map(|p| p.samples.len()).sum::<usize>(), 1);
        assert_eq!(profiles.len(), 2, "two people may share a display name");

        VoiceprintRepository::set_link(&pool, &meeting, "Speaker 1", &anna, STATUS_CONFIRMED, None).await.unwrap();
        VoiceprintRepository::replace_suggestions(&pool, &meeting, &[("Speaker 2".into(), other_anna.clone(), 0.7), ("Speaker 1".into(), other_anna.clone(), 0.7)], true)
            .await
            .unwrap();
        TranscriptsRepository::rename_speaker(&pool, &meeting, "Speaker 1", "Анна").await.unwrap();
        let links = VoiceprintRepository::meeting_links(&pool, &meeting).await.unwrap();
        assert_eq!(
            links.iter().map(|l| (l.speaker.as_str(), l.status.as_str(), l.profile_id == anna)).collect::<Vec<_>>(),
            [("Speaker 2", STATUS_SUGGESTED, false), ("Анна", STATUS_CONFIRMED, true)],
            "a suggestion never replaces a confirmed link"
        );
        // A manual name replaces a pending suggestion.
        TranscriptsRepository::rename_speaker(&pool, &meeting, "Speaker 2", "Игорь").await.unwrap();
        assert_eq!(VoiceprintRepository::meeting_links(&pool, &meeting).await.unwrap().len(), 1);

        // Re-diarization with the same intervals keeps the confirmed name;
        // renumbered speakers drop suggestions and rejections.
        VoiceprintRepository::set_link(&pool, &meeting, "Speaker 3", &other_anna, STATUS_REJECTED, None).await.unwrap();
        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &turns).await.unwrap();
        let links = VoiceprintRepository::meeting_links(&pool, &meeting).await.unwrap();
        assert_eq!(links.iter().map(|l| l.speaker.as_str()).collect::<Vec<_>>(), ["Анна"]);
        // Different intervals (another engine): the confirmed label is gone.
        TranscriptsRepository::apply_speaker_turns(&pool, &meeting, &[turn(0.0, 9.0, "Speaker 1"), turn(9.0, 30.0, "Speaker 2")])
            .await
            .unwrap();
        assert!(VoiceprintRepository::meeting_links(&pool, &meeting).await.unwrap().is_empty());

        // Forgetting a voice removes its samples and links only.
        VoiceprintRepository::delete_profile(&pool, &anna).await.unwrap();
        assert_eq!(VoiceprintRepository::list_profiles(&pool, EXTRACTOR).await.unwrap().len(), 1);
    }
}
