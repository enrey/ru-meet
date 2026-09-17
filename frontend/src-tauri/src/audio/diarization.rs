//! Offline speaker diarization with a replaceable engine backend.
//!
//! Engines are PyAnnote segmentation + WeSpeaker embeddings and NVIDIA
//! Sortformer v2. The application-facing trait deliberately does not expose
//! either implementation's types.

use super::decoder::decode_audio_file;
use super::recording_saver::DiarizationTarget;
use crate::{database::repositories::transcript::TranscriptsRepository, state::AppState};
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use parakeet_rs::sortformer::{DiarizationConfig, Sortformer};
use polyvoice::{ModelRegistry, Pipeline, Profile, SampleRate};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf, sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;
use tokio::io::AsyncWriteExt;

const PYANNOTE_WESPEAKER_ENGINE: &str = "pyannote-wespeaker";
const SORTFORMER_V2_ENGINE: &str = "nvidia-sortformer-v2";
const SORTFORMER_V2_MODEL: &str = "diar_streaming_sortformer_4spk-v2.onnx";
// NVIDIA publishes the checkpoint; this is its ONNX conversion maintained by
// parakeet-rs, which is the native Rust inference implementation used below.
const SORTFORMER_V2_MODEL_URL: &str = "https://huggingface.co/altunenes/parakeet-rs/resolve/main/diar_streaming_sortformer_4spk-v2.onnx";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationSettings {
    pub enabled: bool,
    pub engine: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationJobStatus {
    pub in_progress: bool,
    pub message: String,
    pub meeting_id: Option<String>,
}

impl Default for DiarizationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            engine: PYANNOTE_WESPEAKER_ENGINE.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerTurn {
    pub start: f64,
    pub end: f64,
    pub speaker: String,
}

pub trait DiarizationEngine: Send + Sync {
    fn id(&self) -> &'static str;
    fn diarize(&self, samples: &[f32]) -> Result<Vec<SpeakerTurn>>;
}

/// Full-recording PyAnnote + WeSpeaker pipeline.  Models are verified by the
/// registry before use and cached under Meetily's application data directory.
struct PyannoteWeSpeakerEngine;

impl PyannoteWeSpeakerEngine {
    fn model_registry() -> Result<ModelRegistry> {
        let root = crate::portable::data_root()
            .cloned()
            .or_else(|| dirs::data_local_dir().map(|path| path.join("Meetily")))
            .ok_or_else(|| anyhow!("Could not resolve the local application-data directory"))?
            .join("models")
            .join("diarization");
        ModelRegistry::with_cache_dir(root).map_err(Into::into)
    }
}

impl DiarizationEngine for PyannoteWeSpeakerEngine {
    fn id(&self) -> &'static str {
        PYANNOTE_WESPEAKER_ENGINE
    }

    fn diarize(&self, samples: &[f32]) -> Result<Vec<SpeakerTurn>> {
        let pipeline = Pipeline::builder()
            .profile(Profile::Balanced)
            .with_models_from(Self::model_registry()?)
            .build()
            .map_err(|error| anyhow!(error))?;
        let sample_rate = SampleRate::new(16_000).expect("16 kHz is a valid sample rate");
        let result = pipeline
            .run(samples, sample_rate)
            .map_err(|error| anyhow!(error))?;
        Ok(result
            .turns
            .into_iter()
            .map(|turn| SpeakerTurn {
                start: turn.time.start,
                end: turn.time.end,
                speaker: format!("Speaker {}", turn.speaker.0 + 1),
            })
            .collect())
    }
}

/// NVIDIA's four-speaker streaming Sortformer v2 model, run offline over the
/// completed recording. Its native Rust wrapper uses the app's shared ONNX
/// Runtime and performs the model's required post-processing.
struct NvidiaSortformerV2Engine;

impl NvidiaSortformerV2Engine {
    fn model_dir() -> Result<PathBuf> {
        Ok(crate::portable::data_root()
            .cloned()
            .or_else(|| dirs::data_local_dir().map(|path| path.join("Meetily")))
            .ok_or_else(|| anyhow!("Could not resolve the local application-data directory"))?
            .join("models")
            .join("diarization")
            .join("sortformer-v2"))
    }

    fn model_path() -> Result<PathBuf> {
        Ok(Self::model_dir()?.join(SORTFORMER_V2_MODEL))
    }
}

impl DiarizationEngine for NvidiaSortformerV2Engine {
    fn id(&self) -> &'static str {
        SORTFORMER_V2_ENGINE
    }

    fn diarize(&self, samples: &[f32]) -> Result<Vec<SpeakerTurn>> {
        let model_path = Self::model_path()?;
        if !model_path.is_file() {
            return Err(anyhow!(
                "NVIDIA Sortformer v2 is not downloaded. Open Settings and select Download models."
            ));
        }
        crate::ensure_onnx_runtime_available()?;
        let mut pipeline = Sortformer::with_config(model_path, None, DiarizationConfig::callhome())
            .map_err(|error| anyhow!("Could not load NVIDIA Sortformer v2: {error}"))?;
        let segments = pipeline
            .diarize(samples.to_vec(), 16_000, 1)
            .map_err(|error| anyhow!("NVIDIA Sortformer v2 diarization failed: {error}"))?;
        Ok(segments
            .into_iter()
            .map(|turn| SpeakerTurn {
                start: turn.start as f64 / 16_000.0,
                end: turn.end as f64 / 16_000.0,
                speaker: format!("Speaker {}", turn.speaker_id + 1),
            })
            .collect())
    }
}

fn engine_for_id(engine: &str) -> Result<Box<dyn DiarizationEngine>> {
    match engine {
        PYANNOTE_WESPEAKER_ENGINE => Ok(Box::new(PyannoteWeSpeakerEngine)),
        SORTFORMER_V2_ENGINE => Ok(Box::new(NvidiaSortformerV2Engine)),
        _ => Err(anyhow!(
            "The selected diarization engine is not available in this build"
        )),
    }
}

static SETTINGS: Lazy<Mutex<DiarizationSettings>> =
    Lazy::new(|| Mutex::new(DiarizationSettings::default()));
static JOB_STATUS: Lazy<Mutex<DiarizationJobStatus>> = Lazy::new(|| {
    Mutex::new(DiarizationJobStatus {
        in_progress: false,
        message: String::new(),
        meeting_id: None,
    })
});
static CANCELLED_RERUNS: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub fn load_diarization_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let store = app
        .store(crate::portable::store_path("diarization-settings.json"))
        .map_err(|error| error.to_string())?;
    if let Some(value) = store.get("settings") {
        let settings: DiarizationSettings =
            serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
        engine_for_id(&settings.engine).map_err(|error| error.to_string())?;
        *SETTINGS
            .lock()
            .map_err(|_| "Diarization settings lock is unavailable")? = settings;
    }
    Ok(())
}

#[tauri::command]
pub fn set_diarization_settings<R: Runtime>(
    app: AppHandle<R>,
    settings: DiarizationSettings,
) -> Result<(), String> {
    engine_for_id(&settings.engine).map_err(|error| error.to_string())?;
    let store = app
        .store(crate::portable::store_path("diarization-settings.json"))
        .map_err(|error| error.to_string())?;
    store.set(
        "settings",
        serde_json::to_value(&settings).map_err(|error| error.to_string())?,
    );
    store.save().map_err(|error| error.to_string())?;
    *SETTINGS
        .lock()
        .map_err(|_| "Diarization settings lock is unavailable")? = settings;
    Ok(())
}

#[tauri::command]
pub fn get_diarization_settings() -> Result<DiarizationSettings, String> {
    SETTINGS
        .lock()
        .map(|settings| settings.clone())
        .map_err(|_| "Diarization settings lock is unavailable".into())
}

#[tauri::command]
pub fn get_diarization_status() -> Result<DiarizationJobStatus, String> {
    JOB_STATUS
        .lock()
        .map(|status| status.clone())
        .map_err(|_| "Diarization status lock is unavailable".into())
}

#[tauri::command]
pub async fn get_meeting_speaker_turns(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<SpeakerTurn>, String> {
    TranscriptsRepository::get_speaker_turns(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| format!("Failed to load speaker timeline: {error}"))
}

#[tauri::command]
pub async fn rename_meeting_speaker(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    old_name: String,
    new_name: String,
) -> Result<(), String> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.chars().count() > 80 {
        return Err("Speaker name must be between 1 and 80 characters".into());
    }
    if old_name == new_name {
        return Ok(());
    }
    TranscriptsRepository::rename_speaker(state.db_manager.pool(), &meeting_id, &old_name, new_name)
        .await
}

#[tauri::command]
pub fn rerun_diarization<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    meeting_folder_path: String,
) -> Result<(), String> {
    let mut status = JOB_STATUS
        .lock()
        .map_err(|_| "Diarization status lock is unavailable")?;
    if status.in_progress {
        return Err("Speaker diarization is already running".into());
    }
    *status = DiarizationJobStatus {
        in_progress: true,
        message: "Identifying speakers…".into(),
        meeting_id: Some(meeting_id.clone()),
    };
    drop(status);
    CANCELLED_RERUNS
        .lock()
        .map_err(|_| "Diarization cancellation lock is unavailable")?
        .remove(&meeting_id);

    let folder = std::path::PathBuf::from(meeting_folder_path);
    let audio_path = [
        "audio.mp4",
        "audio.m4a",
        "audio.wav",
        "audio.mp3",
        "audio.flac",
        "audio.ogg",
        "audio.webm",
    ]
    .iter()
    .map(|name| folder.join(name))
    .find(|path| path.is_file())
    .ok_or_else(|| "No recording audio was found for this meeting".to_string())?;
    let pool = state.db_manager.pool().clone();
    let engine = SETTINGS
        .lock()
        .map_err(|_| "Diarization settings lock is unavailable")?
        .engine
        .clone();
    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"stage":"processing", "message":"Identifying speakers…", "meetingId": meeting_id}),
    );

    tauri::async_runtime::spawn(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<Vec<SpeakerTurn>> {
            let decoded = decode_audio_file(&audio_path)?;
            engine_for_id(&engine)?.diarize(&decoded.to_whisper_format())
        })
        .await;

        match result {
            _ if take_rerun_cancellation(&meeting_id) => {
                finish_cancelled_rerun(&app, &meeting_id);
            }
            Ok(Ok(turns)) => {
                match TranscriptsRepository::apply_speaker_turns(&pool, &meeting_id, &turns).await {
                    Ok(()) => {
                        if let Ok(mut status) = JOB_STATUS.lock() {
                            *status = DiarizationJobStatus {
                                in_progress: false,
                                message: "Speaker labels are ready".into(),
                                meeting_id: Some(meeting_id.clone()),
                            };
                        }
                        let _ = app.emit("diarization-complete", serde_json::json!({
                        "meetingId": meeting_id,
                        "speakers": turns.iter().map(|turn| &turn.speaker).collect::<std::collections::BTreeSet<_>>().len(),
                        "labels": [],
                    }));
                        let _ = app.emit(
                            "diarization-labels-saved",
                            serde_json::json!({"meetingId": meeting_id}),
                        );
                    }
                    Err(error) => emit_rerun_error(&app, &meeting_id, error.to_string()),
                }
            }
            Ok(Err(error)) => emit_rerun_error(&app, &meeting_id, error.to_string()),
            Err(error) => emit_rerun_error(&app, &meeting_id, error.to_string()),
        }
    });
    Ok(())
}

#[tauri::command]
pub fn cancel_diarization<R: Runtime>(app: AppHandle<R>, meeting_id: String) -> Result<(), String> {
    let status = JOB_STATUS
        .lock()
        .map_err(|_| "Diarization status lock is unavailable")?;
    if !status.in_progress || status.meeting_id.as_deref() != Some(meeting_id.as_str()) {
        return Err("No diarization job is running for this meeting".into());
    }
    drop(status);
    CANCELLED_RERUNS
        .lock()
        .map_err(|_| "Diarization cancellation lock is unavailable")?
        .insert(meeting_id.clone());
    let _ = app.emit(
        "diarization-cancelling",
        serde_json::json!({"meetingId": meeting_id, "message": "Stopping speaker diarization…"}),
    );
    Ok(())
}

fn take_rerun_cancellation(meeting_id: &str) -> bool {
    CANCELLED_RERUNS
        .lock()
        .map(|mut cancelled| cancelled.remove(meeting_id))
        .unwrap_or(false)
}

fn finish_cancelled_rerun<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) {
    if let Ok(mut status) = JOB_STATUS.lock() {
        *status = DiarizationJobStatus {
            in_progress: false,
            message: "Speaker diarization stopped".into(),
            meeting_id: Some(meeting_id.to_string()),
        };
    }
    let _ = app.emit(
        "diarization-cancelled",
        serde_json::json!({"meetingId": meeting_id}),
    );
}

fn emit_rerun_error<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, error: String) {
    log::warn!("Diarization failed: {error}");
    if let Ok(mut status) = JOB_STATUS.lock() {
        *status = DiarizationJobStatus {
            in_progress: false,
            message: "Speaker diarization failed".into(),
            meeting_id: Some(meeting_id.to_string()),
        };
    }
    let _ = app.emit(
        "diarization-rerun-error",
        serde_json::json!({"meetingId": meeting_id}),
    );
    let _ = app.emit("diarization-error", error);
}

/// Download and validate the bundle for the currently selected engine before a
/// recording starts.
#[tauri::command]
pub async fn download_diarization_models<R: Runtime>(
    app: AppHandle<R>,
    engine: Option<String>,
) -> Result<(), String> {
    let engine = match engine {
        Some(engine) => engine,
        None => SETTINGS
            .lock()
            .map_err(|_| "Diarization settings lock is unavailable")?
            .engine
            .clone(),
    };
    engine_for_id(&engine).map_err(|error| error.to_string())?;
    app.emit(
        "diarization-download-progress",
        serde_json::json!({ "progress": 0, "message": "Preparing speaker models…", "engine": engine }),
    )
    .map_err(|error| error.to_string())?;
    let result = match engine.as_str() {
        PYANNOTE_WESPEAKER_ENGINE => tokio::task::spawn_blocking(|| -> Result<()> {
            Pipeline::builder()
                .profile(Profile::Balanced)
                .with_models_from(PyannoteWeSpeakerEngine::model_registry()?)
                .build()
                .map_err(|error| anyhow!(error))?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?,
        SORTFORMER_V2_ENGINE => download_sortformer_v2_model(app.clone()).await,
        _ => Err(anyhow!(
            "The selected diarization engine is not available in this build"
        )),
    };
    result.map_err(|error| error.to_string())?;
    app.emit(
        "diarization-download-progress",
        serde_json::json!({ "progress": 100, "message": "Speaker models are ready", "engine": engine }),
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

async fn download_sortformer_v2_model<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    let model_dir = NvidiaSortformerV2Engine::model_dir()?;
    let model_path = NvidiaSortformerV2Engine::model_path()?;
    if model_path.is_file() {
        return Ok(());
    }
    tokio::fs::create_dir_all(&model_dir).await?;
    let temporary_path = model_dir.join(format!(".{SORTFORMER_V2_MODEL}.downloading"));
    let _ = tokio::fs::remove_file(&temporary_path).await;
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(900))
        .build()?
        .get(SORTFORMER_V2_MODEL_URL)
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut output = tokio::fs::File::create(&temporary_path).await?;
    let mut downloaded = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        output.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        let progress = total
            .map(|size| ((downloaded * 100 / size).min(99)) as u8)
            .unwrap_or(0);
        let _ = app.emit(
            "diarization-download-progress",
            serde_json::json!({
                "progress": progress,
                "message": "Downloading NVIDIA Sortformer v2…",
                "engine": SORTFORMER_V2_ENGINE,
            }),
        );
    }
    output.flush().await?;
    drop(output);
    if downloaded == 0 {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(anyhow!("NVIDIA Sortformer v2 download was empty"));
    }
    tokio::fs::rename(&temporary_path, &model_path).await?;
    Ok(())
}

pub fn spawn_diarization_task<R: Runtime>(
    app: AppHandle<R>,
    target: DiarizationTarget,
    audio_path: String,
) {
    let settings = SETTINGS
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    if !settings.enabled {
        return;
    }
    let engine = settings.engine;

    if let Ok(mut status) = JOB_STATUS.lock() {
        *status = DiarizationJobStatus {
            in_progress: true,
            message: "Identifying speakers…".into(),
            meeting_id: None,
        };
    }

    let path = std::path::PathBuf::from(audio_path);
    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"stage":"processing", "message":"Identifying speakers…"}),
    );
    tauri::async_runtime::spawn(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<Vec<SpeakerTurn>> {
            let decoded = decode_audio_file(&path)?;
            let audio = decoded.to_whisper_format();
            engine_for_id(&engine)?.diarize(&audio)
        })
        .await;
        match result {
            Ok(Ok(turns)) => {
                let labels: Vec<_> = target.apply_speaker_turns(&turns).into_iter().map(|(sequence_id, speaker)| serde_json::json!({"sequenceId": sequence_id, "speaker": speaker})).collect();
                let _ = app.emit(
                "diarization-complete",
                serde_json::json!({
                    "speakers": turns.iter().map(|turn| &turn.speaker).collect::<std::collections::BTreeSet<_>>().len(),
                    "labels": labels,
                    "turns": turns,
                }),
            );
                if let Ok(mut status) = JOB_STATUS.lock() {
                    *status = DiarizationJobStatus {
                        in_progress: false,
                        message: "Speaker labels are ready".into(),
                        meeting_id: None,
                    };
                }
            }
            Ok(Err(error)) => {
                log::warn!("Diarization failed: {error}");
                let _ = app.emit("diarization-error", error.to_string());
                if let Ok(mut status) = JOB_STATUS.lock() {
                    *status = DiarizationJobStatus {
                        in_progress: false,
                        message: "Speaker diarization failed".into(),
                        meeting_id: None,
                    };
                }
            }
            Err(error) => {
                log::warn!("Diarization worker failed: {error}");
                let _ = app.emit("diarization-error", error.to_string());
                if let Ok(mut status) = JOB_STATUS.lock() {
                    *status = DiarizationJobStatus {
                        in_progress: false,
                        message: "Speaker diarization failed".into(),
                        meeting_id: None,
                    };
                }
            }
        }
    });
}
