//! Background processing of a saved recording: speakers, summary, reading
//! aloud - whatever is missing or failed, one after another, without any page
//! being open. Each step follows the settings: speakers only when diarization
//! is on, the reading only when it is installed and on.
//!
//! The steps themselves are the existing background jobs; this only chains
//! them, deciding from the same stages the library shows.

use crate::api::api::{meeting_snapshot, MeetingSnapshot, StageState};
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// How often a running step is checked on.
const POLL: Duration = Duration::from_secs(2);
/// The template used when no page chose one.
const DEFAULT_TEMPLATE: &str = "standard_meeting";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Speakers,
    Summary,
    Speech,
}

static RUNS: Lazy<Mutex<HashMap<String, Step>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// The step a meeting's processing is on, if it is being processed.
pub fn current_step(meeting_id: &str) -> Option<Step> {
    RUNS.lock().ok()?.get(meeting_id).copied()
}

fn set_step<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, step: Step) {
    if let Ok(mut runs) = RUNS.lock() {
        runs.insert(meeting_id.to_string(), step);
    }
    let _ = app.emit(
        "meeting-processing",
        serde_json::json!({ "meetingId": meeting_id, "step": step }),
    );
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
enum Outcome {
    /// Every applicable step is done, apart from `failed`.
    Done { failed: Vec<Step> },
    /// Nothing was missing.
    NothingToDo,
    /// A step was stopped by the user; the rest were not run.
    Stopped { step: Step },
    /// The summary could not be made, so nothing after it was run.
    Failed { step: Step, message: String },
}

/// Process the meeting in the background. Errors only when it cannot start.
#[tauri::command]
pub fn process_meeting<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> Result<(), String> {
    start(app, meeting_id, true)
}

/// `requested`: the user asked for it, so even "nothing to do" is reported.
pub(crate) fn start<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    requested: bool,
) -> Result<(), String> {
    {
        let mut runs = RUNS.lock().map_err(|_| "The processing lock is unavailable")?;
        if runs.contains_key(&meeting_id) {
            return Err("This recording is already being processed".into());
        }
        runs.insert(meeting_id.clone(), Step::Speakers);
    }
    tauri::async_runtime::spawn(async move {
        let outcome = match run(&app, &meeting_id).await {
            Ok(outcome) => outcome,
            Err(message) => Outcome::Failed { step: current_step(&meeting_id).unwrap_or(Step::Speakers), message },
        };
        log::info!("Processing of {meeting_id} ended: {outcome:?}");
        if let Ok(mut runs) = RUNS.lock() {
            runs.remove(&meeting_id);
        }
        let mut payload = serde_json::to_value(&outcome).unwrap_or_default();
        payload["meetingId"] = serde_json::json!(meeting_id);
        payload["requested"] = serde_json::json!(requested);
        let _ = app.emit("meeting-processed", payload);
    });
    Ok(())
}

async fn snapshot<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<MeetingSnapshot, String> {
    meeting_snapshot(app, pool, meeting_id)
        .await?
        .ok_or_else(|| "The meeting was not found".to_string())
}

async fn run<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<Outcome, String> {
    let pool = app
        .try_state::<AppState>()
        .ok_or("The database is unavailable")?
        .db_manager
        .pool()
        .clone();
    let first = snapshot(app, &pool, meeting_id).await?;
    let diarization_on = crate::audio::diarization::get_diarization_settings()
        .is_ok_and(|settings| settings.enabled);
    let can_speak = crate::audio::tts::commands::can_speak(app);

    let speakers = first.stages.speakers.state;
    let need_speakers = (diarization_on && first.folder_path.is_some() && speakers != StageState::Done)
        || speakers == StageState::Running;
    let need_summary = first.stages.summary.state != StageState::Done;
    let need_speech = can_speak && first.stages.speech.state != StageState::Done;
    if !need_speakers && !need_summary && !need_speech {
        return Ok(Outcome::NothingToDo);
    }

    let mut failed = Vec::new();
    if need_speakers {
        set_step(app, meeting_id, Step::Speakers);
        match run_speakers(app, &pool, meeting_id, &first).await {
            Finished::Done => {}
            Finished::Failed => failed.push(Step::Speakers),
            Finished::Stopped => return Ok(Outcome::Stopped { step: Step::Speakers }),
        }
    }

    if need_summary {
        set_step(app, meeting_id, Step::Summary);
        match run_summary(app, &pool, meeting_id).await {
            Ok(Finished::Done) => {}
            Ok(Finished::Stopped) => return Ok(Outcome::Stopped { step: Step::Summary }),
            Ok(Finished::Failed) => {
                return Ok(Outcome::Failed { step: Step::Summary, message: String::new() })
            }
            Err(message) => return Ok(Outcome::Failed { step: Step::Summary, message }),
        }
    }

    if can_speak {
        // A new summary starts its reading by itself when that is turned on.
        let speech = snapshot(app, &pool, meeting_id).await?.stages.speech.state;
        if speech != StageState::Done {
            set_step(app, meeting_id, Step::Speech);
            match run_speech(app, &pool, meeting_id, speech == StageState::Running).await {
                Finished::Done => {}
                Finished::Failed => failed.push(Step::Speech),
                Finished::Stopped => return Ok(Outcome::Stopped { step: Step::Speech }),
            }
        }
    }
    Ok(Outcome::Done { failed })
}

enum Finished {
    Done,
    Failed,
    Stopped,
}

async fn run_speakers<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    snapshot: &MeetingSnapshot,
) -> Finished {
    use crate::audio::diarization::{meeting_job, start_for_folder, DIARIZATION_FAILED};

    if snapshot.stages.speakers.state != StageState::Running {
        let Some(folder) = snapshot.folder_path.as_deref() else {
            return Finished::Failed;
        };
        if let Err(error) = start_for_folder(
            app.clone(),
            pool.clone(),
            meeting_id.to_string(),
            std::path::Path::new(folder),
        ) {
            log::warn!("Processing {meeting_id}: speakers could not start: {error}");
            return Finished::Failed;
        }
    }
    loop {
        tokio::time::sleep(POLL).await;
        match meeting_job(meeting_id) {
            Some(job) if job.in_progress => continue,
            Some(job) if job.message == DIARIZATION_FAILED => return Finished::Failed,
            Some(job) if job.message == "Speaker diarization stopped" => return Finished::Stopped,
            _ => return Finished::Done,
        }
    }
}

async fn run_summary<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<Finished, String> {
    if snapshot(app, pool, meeting_id).await?.stages.summary.state != StageState::Running {
        let model = SettingsRepository::get_model_config(pool)
            .await
            .map_err(|error| error.to_string())?
            .filter(|config| !config.provider.trim().is_empty() && !config.model.trim().is_empty())
            .ok_or("Choose a summary model in the settings")?;
        let (text, texts) = transcript_for_summary(pool, meeting_id).await?;
        if text.trim().is_empty() {
            return Err("The recording has no transcript to summarize".into());
        }
        let language = summary_language(pool, meeting_id, &texts).await;
        crate::summary::commands::start_summary(
            app.clone(),
            pool.clone(),
            meeting_id.to_string(),
            text,
            model.provider,
            model.model,
            None,
            None,
            None,
            Some(DEFAULT_TEMPLATE.to_string()),
            language,
        )
        .await?;
    }
    loop {
        tokio::time::sleep(POLL).await;
        match snapshot(app, pool, meeting_id).await?.stages.summary.state {
            StageState::Running => continue,
            StageState::Done => return Ok(Finished::Done),
            StageState::Failed => return Ok(Finished::Failed),
            // Cancelled: the process is gone without a result.
            StageState::None => return Ok(Finished::Stopped),
        }
    }
}

async fn run_speech<R: Runtime>(
    app: &AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    already_running: bool,
) -> Finished {
    if !already_running {
        if let Err(error) =
            crate::audio::tts::tts_prepare_summary_audio(app.clone(), meeting_id.to_string())
        {
            log::warn!("Processing {meeting_id}: reading aloud could not start: {error}");
            return Finished::Failed;
        }
    }
    loop {
        tokio::time::sleep(POLL).await;
        match snapshot(app, pool, meeting_id).await.map(|s| s.stages.speech.state) {
            Ok(StageState::Running) => continue,
            Ok(StageState::Done) => return Finished::Done,
            Ok(StageState::None) => return Finished::Stopped,
            Ok(StageState::Failed) | Err(_) => return Finished::Failed,
        }
    }
}

/// The transcript as the summary reads it: one line per speaker turn,
/// consecutive segments of the same speaker merged - as the meeting page
/// builds it - plus the plain segment texts for language detection.
async fn transcript_for_summary(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<(String, Vec<String>), String> {
    let (transcripts, _) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, i64::MAX, 0)
            .await
            .map_err(|error| error.to_string())?;
    let mut turns: Vec<(String, Vec<String>)> = Vec::new();
    for transcript in &transcripts {
        let speaker = transcript.speaker.as_deref().unwrap_or("").trim().to_string();
        match turns.last_mut() {
            Some((last, texts)) if !speaker.is_empty() && *last == speaker => {
                texts.push(transcript.transcript.clone())
            }
            _ => turns.push((speaker, vec![transcript.transcript.clone()])),
        }
    }
    let text = turns
        .iter()
        .map(|(speaker, texts)| {
            if speaker.is_empty() {
                texts.join(" ")
            } else {
                format!("{speaker}: {}", texts.join(" "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let texts = transcripts.into_iter().map(|t| t.transcript).collect();
    Ok((text, texts))
}

/// The meeting's chosen summary language, else the cached detection, else a
/// fresh detection (cached for next time) - the meeting page's order.
async fn summary_language(pool: &sqlx::SqlitePool, meeting_id: &str, texts: &[String]) -> Option<String> {
    use crate::summary::metadata::{
        read_detected_summary_language_from_metadata, read_summary_language_from_metadata,
        write_detected_summary_language_to_metadata,
    };
    let folder = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .ok()
        .flatten()
        .and_then(|meeting| meeting.folder_path)
        .filter(|path| !path.trim().is_empty())
        .map(std::path::PathBuf::from);
    if let Some(folder) = &folder {
        if let Ok(Some(language)) = read_summary_language_from_metadata(folder) {
            return Some(language);
        }
        if let Ok(Some(language)) = read_detected_summary_language_from_metadata(folder) {
            return Some(language);
        }
    }
    let detected = crate::summary::language_detection::detect_summary_language(texts).language;
    if let (Some(folder), Some(language)) = (&folder, &detected) {
        let _ = write_detected_summary_language_to_metadata(folder, Some(language));
    }
    detected
}
