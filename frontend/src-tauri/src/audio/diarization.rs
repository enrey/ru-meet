//! Offline speaker diarization with a replaceable engine backend.
//!
//! Engines are speakrs and polyvoice - two independent implementations of
//! PyAnnote segmentation + WeSpeaker embeddings - and NVIDIA Sortformer v2. The
//! application-facing trait deliberately does not expose any implementation's
//! types.

use super::decoder::decode_audio_file;
use super::recording_saver::DiarizationTarget;
use crate::{database::repositories::transcript::TranscriptsRepository, state::AppState};
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use parakeet_rs::sortformer::{DiarizationConfig, Sortformer};
use polyvoice::{ModelRegistry, Pipeline, Profile, SampleRate};
use serde::{Deserialize, Serialize};
use speakrs::{ExecutionMode, OwnedDiarizationPipeline};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;
use tokio::io::AsyncWriteExt;

/// Longest recording polyvoice will diarize, in 16 kHz samples (12 hours).
/// Bounded rather than unlimited so a corrupt duration still cannot make the
/// pipeline allocate without limit.
const MAX_DIARIZATION_SAMPLES: usize = 16_000 * 3_600 * 12;

const SPEAKRS_ENGINE: &str = "speakrs-pyannote-wespeaker";
const PYANNOTE_WESPEAKER_ENGINE: &str = "pyannote-wespeaker";
const SORTFORMER_V2_ENGINE: &str = "nvidia-sortformer-v2";
const SORTFORMER_V2_MODEL: &str = "diar_streaming_sortformer_4spk-v2.onnx";
const PYANNOTE_WESPEAKER_MODEL_FILES: [&str; 8] = [
    "powerset_int8.onnx",
    "resnet34_int8.onnx",
    "plda_lda.npy",
    "plda_mean1.npy",
    "plda_mean2.npy",
    "plda_mu.npy",
    "plda_phi_computed.npy",
    "plda_transform.npy",
];
// speakrs pins this revision of its model repository, so the app downloads the
// exact bundle the linked crate version expects. Sizes come from that revision
// and double as the integrity check, since HuggingFace serves no per-file hash
// on the resolve endpoint.
const SPEAKRS_MODEL_REVISION: &str = "a785ebdbe6313868088c36c93d9efa71c470bd34";
const SPEAKRS_MODEL_FILES: [(&str, u64); 10] = [
    ("segmentation-3.0.onnx", 5_916_308),
    ("wespeaker-voxceleb-resnet34.onnx", 26_894_815),
    ("wespeaker-voxceleb-resnet34.onnx.data", 26_673_152),
    ("wespeaker-voxceleb-resnet34.min_num_samples.txt", 4),
    ("plda_lda.npy", 131_200),
    ("plda_tr.npy", 131_200),
    ("plda_mu.npy", 1_152),
    ("plda_psi.npy", 1_152),
    ("plda_mean1.npy", 2_176),
    ("plda_mean2.npy", 640),
];
// NVIDIA publishes the checkpoint; this is its ONNX conversion maintained by
// parakeet-rs, which is the native Rust inference implementation used below.
const SORTFORMER_V2_MODEL_URL: &str = "https://huggingface.co/altunenes/parakeet-rs/resolve/main/diar_streaming_sortformer_4spk-v2.onnx";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationSettings {
    pub enabled: bool,
    pub engine: String,
    /// Merge barely-heard speakers into one `Others` label. Defaulted rather
    /// than required so settings written before this existed still load.
    #[serde(default = "default_collapse_minor_speakers")]
    pub collapse_minor_speakers: bool,
}

fn default_collapse_minor_speakers() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarizationModelStatus {
    pub engine: String,
    pub ready: bool,
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
            collapse_minor_speakers: default_collapse_minor_speakers(),
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

/// Full pyannote `community-1` pipeline (segmentation, powerset decode,
/// WeSpeaker embeddings, PLDA, VBx) as implemented by the `speakrs` crate. It
/// runs on the application's shared ONNX Runtime and reads a model bundle the
/// app downloads itself, so no HuggingFace cache is involved at runtime.
struct SpeakrsEngine;

impl SpeakrsEngine {
    fn model_dir() -> Result<PathBuf> {
        Ok(crate::portable::data_root()
            .cloned()
            .or_else(|| dirs::data_local_dir().map(|path| path.join("Meetily")))
            .ok_or_else(|| anyhow!("Could not resolve the local application-data directory"))?
            .join("models")
            .join("diarization")
            .join("speakrs"))
    }

    fn is_ready() -> bool {
        let Ok(dir) = Self::model_dir() else {
            return false;
        };
        SPEAKRS_MODEL_FILES.iter().all(|(name, size)| {
            std::fs::metadata(dir.join(name))
                .map(|metadata| metadata.is_file() && metadata.len() == *size)
                .unwrap_or(false)
        })
    }
}

/// speakrs emits `SPEAKER_00`-style labels; the rest of the app stores the
/// same `Speaker N` names every other engine produces.
fn speakrs_speaker_label(label: &str) -> String {
    label
        .rsplit('_')
        .next()
        .and_then(|index| index.parse::<usize>().ok())
        .map(|index| format!("Speaker {}", index + 1))
        .unwrap_or_else(|| label.to_string())
}

impl DiarizationEngine for SpeakrsEngine {
    fn id(&self) -> &'static str {
        SPEAKRS_ENGINE
    }

    fn diarize(&self, samples: &[f32]) -> Result<Vec<SpeakerTurn>> {
        if !Self::is_ready() {
            return Err(anyhow!(
                "speakrs is not downloaded. Open Settings and select Download models."
            ));
        }
        crate::ensure_onnx_runtime_available()?;

        // Loading the FP32 WeSpeaker ResNet34 is itself slow enough to look
        // like a stall, so time the two phases separately.
        let loading = Instant::now();
        let mut pipeline = OwnedDiarizationPipeline::from_dir(Self::model_dir()?, ExecutionMode::Cpu)
            .map_err(|error| anyhow!("Could not load speakrs: {error}"))?;
        log::info!(
            "speakrs: models loaded in {:.1}s, starting inference",
            loading.elapsed().as_secs_f64()
        );

        let inference = Instant::now();
        let result = pipeline
            .run(samples)
            .map_err(|error| anyhow!("speakrs diarization failed: {error}"))?;
        log::info!(
            "speakrs: inference finished in {:.1}s, {} raw segments",
            inference.elapsed().as_secs_f64(),
            result.segments.len()
        );
        // These are merged per-speaker turns and may overlap during crosstalk;
        // `apply_speaker_turns` resolves that by longest overlap, the same way
        // it does for the other engines.
        Ok(result
            .segments
            .into_iter()
            .map(|segment| SpeakerTurn {
                start: segment.start,
                end: segment.end,
                speaker: speakrs_speaker_label(&segment.speaker),
            })
            .collect())
    }
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
            // polyvoice defaults to refusing anything over an hour, a guard
            // aimed at untrusted buffers reaching its C FFI and Python
            // bindings. Our audio is a recording the app just made and decoded
            // in-process, so raise it rather than fail a long meeting outright.
            .max_audio_samples(MAX_DIARIZATION_SAMPLES)
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

/// A speaker is "minor" only if it clears both bars: under this share of all
/// attributed speech, *and* under the absolute ceiling below. A share alone
/// scales badly - 1% of a two-hour meeting is 72 seconds, which is a real
/// participant, while 1% of a ten-minute one is six seconds.
const MINOR_SPEAKER_SHARE: f64 = 0.01;
const MINOR_SPEAKER_MAX_SECONDS: f64 = 30.0;
/// Never collapse the busiest speakers, whatever the arithmetic says, so a
/// badly fragmented diarization cannot turn the whole meeting into `Others`.
const MIN_KEPT_SPEAKERS: usize = 2;
const OTHERS_SPEAKER: &str = "Others";

/// Merge speakers with a negligible amount of speech into a single `Others`.
///
/// Diarization on a long meeting routinely invents a tail of speakers that say
/// one short phrase each; eleven speakers where four spoke is worse than
/// useless in the timeline. This is deliberately lossy - the original labels
/// are not stored - because re-running diarization restores them.
fn collapse_minor_speakers(turns: Vec<SpeakerTurn>) -> Vec<SpeakerTurn> {
    let mut spoken: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for turn in &turns {
        *spoken.entry(turn.speaker.as_str()).or_insert(0.0) += (turn.end - turn.start).max(0.0);
    }
    let total: f64 = spoken.values().sum();
    if total <= 0.0 || spoken.len() <= MIN_KEPT_SPEAKERS {
        return turns;
    }

    // Rank by speech time so the busiest speakers can be exempted outright.
    let mut ranked: Vec<(&str, f64)> = spoken.iter().map(|(name, secs)| (*name, *secs)).collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(right.0)));

    let minor: HashSet<String> = ranked
        .iter()
        .skip(MIN_KEPT_SPEAKERS)
        .filter(|(_, secs)| *secs / total < MINOR_SPEAKER_SHARE && *secs < MINOR_SPEAKER_MAX_SECONDS)
        .map(|(name, _)| (*name).to_owned())
        .collect();

    // Collapsing one speaker just renames them and loses which one they were.
    if minor.len() < 2 {
        return turns;
    }
    log::info!(
        "Diarization: merging {} of {} speakers into \"{OTHERS_SPEAKER}\" (each under {:.0}% and {MINOR_SPEAKER_MAX_SECONDS:.0}s of speech)",
        minor.len(),
        ranked.len(),
        MINOR_SPEAKER_SHARE * 100.0,
    );

    let mut collapsed: Vec<SpeakerTurn> = turns
        .into_iter()
        .map(|turn| {
            if minor.contains(&turn.speaker) {
                SpeakerTurn {
                    speaker: OTHERS_SPEAKER.to_owned(),
                    ..turn
                }
            } else {
                turn
            }
        })
        .collect();

    // Relabelling can leave several `Others` slivers back to back, which would
    // show up as separate blocks on the timeline. Join only the ones that touch
    // or overlap, so merging never swallows another speaker's time in between.
    collapsed.sort_by(|left, right| left.start.total_cmp(&right.start));
    let mut merged: Vec<SpeakerTurn> = Vec::with_capacity(collapsed.len());
    for turn in collapsed {
        match merged.last_mut() {
            Some(previous)
                if previous.speaker == OTHERS_SPEAKER
                    && turn.speaker == OTHERS_SPEAKER
                    && turn.start <= previous.end =>
            {
                previous.end = previous.end.max(turn.end);
            }
            _ => merged.push(turn),
        }
    }
    merged
}

/// Run one engine and apply the app-level speaker post-processing, so every
/// caller gets turns shaped the same way.
fn diarize_with_engine(engine: &str, samples: &[f32], collapse: bool) -> Result<Vec<SpeakerTurn>> {
    let turns = engine_for_id(engine)?.diarize(samples)?;
    Ok(if collapse {
        collapse_minor_speakers(turns)
    } else {
        turns
    })
}

fn engine_for_id(engine: &str) -> Result<Box<dyn DiarizationEngine>> {
    match engine {
        SPEAKRS_ENGINE => Ok(Box::new(SpeakrsEngine)),
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
pub fn get_diarization_model_statuses() -> Result<Vec<DiarizationModelStatus>, String> {
    let pyannote_root = PyannoteWeSpeakerEngine::model_registry()
        .map_err(|error| error.to_string())?
        .cache_dir()
        .to_path_buf();
    let pyannote_ready = PYANNOTE_WESPEAKER_MODEL_FILES.iter().all(|file| {
        std::fs::metadata(pyannote_root.join(file))
            .map(|metadata| metadata.is_file() && metadata.len() > 0)
            .unwrap_or(false)
    });
    let sortformer_path =
        NvidiaSortformerV2Engine::model_path().map_err(|error| error.to_string())?;
    let sortformer_ready = std::fs::metadata(sortformer_path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false);

    Ok(vec![
        DiarizationModelStatus {
            engine: PYANNOTE_WESPEAKER_ENGINE.into(),
            ready: pyannote_ready,
        },
        DiarizationModelStatus {
            engine: SPEAKRS_ENGINE.into(),
            ready: SpeakrsEngine::is_ready(),
        },
        DiarizationModelStatus {
            engine: SORTFORMER_V2_ENGINE.into(),
            ready: sortformer_ready,
        },
    ])
}

#[tauri::command]
pub fn get_diarization_status() -> Result<DiarizationJobStatus, String> {
    JOB_STATUS
        .lock()
        .map(|status| status.clone())
        .map_err(|_| "Diarization status lock is unavailable".into())
}

#[tauri::command]
pub async fn get_meeting_speaker_turns<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<SpeakerTurn>, String> {
    let pool = state.db_manager.pool();

    // Self-heal meetings whose turns were saved without reaching the transcript
    // rows; otherwise the timeline shows speakers the transcript never got, and
    // renaming one of them appears to do nothing.
    match TranscriptsRepository::backfill_speakers_from_turns(pool, &meeting_id).await {
        Ok(0) => {}
        Ok(updated) => {
            log::info!(
                "Restored {updated} transcript speaker labels for meeting {meeting_id} from stored turns"
            );
            crate::audio::transcript_export::export_meeting_transcripts_logged(pool, &meeting_id)
                .await;
            let _ = app.emit(
                "transcript-speakers-updated",
                serde_json::json!({ "meetingId": meeting_id }),
            );
        }
        Err(error) => {
            log::warn!("Could not restore speaker labels for meeting {meeting_id}: {error}")
        }
    }

    TranscriptsRepository::get_speaker_turns(pool, &meeting_id)
        .await
        .map_err(|error| format!("Failed to load speaker timeline: {error}"))
}

/// Returns `true` when the new name already belonged to another speaker, so
/// the two were merged into one rather than simply relabelled.
#[tauri::command]
pub async fn rename_meeting_speaker(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    old_name: String,
    new_name: String,
) -> Result<bool, String> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.chars().count() > 80 {
        return Err("Speaker name must be between 1 and 80 characters".into());
    }
    if old_name == new_name {
        return Ok(false);
    }
    let pool = state.db_manager.pool();
    let merged =
        TranscriptsRepository::rename_speaker(pool, &meeting_id, &old_name, new_name).await?;
    crate::audio::transcript_export::export_meeting_transcripts_logged(pool, &meeting_id).await;
    Ok(merged)
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
    let (engine, collapse) = {
        let settings = SETTINGS
            .lock()
            .map_err(|_| "Diarization settings lock is unavailable")?;
        (settings.engine.clone(), settings.collapse_minor_speakers)
    };
    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"stage":"processing", "message":"Identifying speakers…", "meetingId": meeting_id}),
    );

    tauri::async_runtime::spawn(async move {
        // Decode first so the heartbeat below can quote the recording's real
        // length, and so a decode failure is reported as such.
        let decoded =
            tokio::task::spawn_blocking(move || decode_for_diarization(&audio_path)).await;
        let samples = match decoded {
            Ok(Ok(samples)) => samples,
            Ok(Err(error)) => return emit_rerun_error(&app, &meeting_id, error.to_string()),
            Err(error) => return emit_rerun_error(&app, &meeting_id, error.to_string()),
        };

        let audio_seconds = samples.len() as f64 / 16_000.0;
        log::info!(
            "Diarization: starting {engine} on {audio_seconds:.0}s of audio for meeting {meeting_id}"
        );
        let started = Instant::now();
        let heartbeat = spawn_progress_heartbeat(
            app.clone(),
            Some(meeting_id.clone()),
            engine.clone(),
            audio_seconds,
        );

        let selected = engine.clone();
        let result =
            tokio::task::spawn_blocking(move || diarize_with_engine(&selected, &samples, collapse))
                .await;
        drop(heartbeat);

        let elapsed = started.elapsed().as_secs_f64();
        log::info!(
            "Diarization: {engine} finished in {elapsed:.1}s ({:.1}x realtime)",
            audio_seconds / elapsed.max(0.001)
        );

        match result {
            _ if take_rerun_cancellation(&meeting_id) => {
                finish_cancelled_rerun(&app, &meeting_id);
            }
            Ok(Ok(turns)) => {
                match TranscriptsRepository::apply_speaker_turns(&pool, &meeting_id, &turns).await {
                    Ok(()) => {
                        crate::audio::transcript_export::export_meeting_transcripts_logged(
                            &pool,
                            &meeting_id,
                        )
                        .await;
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

/// Stops the heartbeat when the diarization call it accompanies returns,
/// including on an early return or a panic.
struct ProgressHeartbeat(Arc<AtomicBool>);

impl Drop for ProgressHeartbeat {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Report elapsed time while a diarization engine runs.
///
/// The engines are single opaque blocking calls - none of them reports how far
/// along it is - and on CPU speakrs can take minutes on a long meeting. Without
/// this the UI sits on one unchanging "Identifying speakers…" line and the logs
/// stay silent, which is indistinguishable from a hang. Report elapsed time
/// against the recording's own length rather than a made-up percentage: it is
/// the honest signal, and it lets the user judge the rate themselves.
fn spawn_progress_heartbeat<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: Option<String>,
    engine: String,
    audio_seconds: f64,
) -> ProgressHeartbeat {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);

    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.tick().await; // the first tick completes immediately

        while !flag.load(Ordering::Relaxed) {
            ticker.tick().await;
            if flag.load(Ordering::Relaxed) {
                break;
            }

            let elapsed = started.elapsed().as_secs();
            let message = format!(
                "Identifying speakers with {engine}… {} elapsed, {} of audio",
                format_elapsed(elapsed),
                format_elapsed(audio_seconds as u64),
            );
            log::info!(
                "Diarization in progress: {} elapsed for {:.0}s of audio ({engine})",
                format_elapsed(elapsed),
                audio_seconds
            );

            if let Ok(mut status) = JOB_STATUS.lock() {
                status.message = message.clone();
            }
            let mut payload = serde_json::json!({ "stage": "processing", "message": message });
            if let Some(meeting_id) = &meeting_id {
                payload["meetingId"] = serde_json::json!(meeting_id);
            }
            let _ = app.emit("diarization-progress", payload);
        }
    });

    ProgressHeartbeat(stop)
}

/// Report a failure from the recording-finalization path, where there is no
/// meeting id yet and the caller returns the error to its own caller.
fn finish_failed_diarization<R: Runtime>(
    app: &AppHandle<R>,
    error: String,
) -> std::result::Result<Vec<SpeakerTurn>, String> {
    log::warn!("Diarization failed: {error}");
    let _ = app.emit("diarization-error", error.clone());
    if let Ok(mut status) = JOB_STATUS.lock() {
        *status = DiarizationJobStatus {
            in_progress: false,
            message: "Speaker diarization failed".into(),
            meeting_id: None,
        };
    }
    Err(error)
}

fn format_elapsed(seconds: u64) -> String {
    match (seconds / 60, seconds % 60) {
        (0, seconds) => format!("{seconds}s"),
        (minutes, seconds) => format!("{minutes}m {seconds:02}s"),
    }
}

/// Decode a recording to the 16 kHz mono buffer every engine expects, and
/// report how long that took separately from diarization itself.
fn decode_for_diarization(path: &std::path::Path) -> Result<Vec<f32>> {
    let started = Instant::now();
    let samples = decode_audio_file(path)?.to_whisper_format();
    log::info!(
        "Diarization: decoded {:.0}s of audio in {:.1}s",
        samples.len() as f64 / 16_000.0,
        started.elapsed().as_secs_f64()
    );
    Ok(samples)
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
        SPEAKRS_ENGINE => download_speakrs_models(app.clone()).await,
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

/// Fetch the pinned speakrs model bundle. Progress is reported against the
/// revision's known byte total, and each file's size is checked before it is
/// moved into place so a truncated download never looks ready.
async fn download_speakrs_models<R: Runtime>(app: AppHandle<R>) -> Result<()> {
    let model_dir = SpeakrsEngine::model_dir()?;
    tokio::fs::create_dir_all(&model_dir).await?;
    let total: u64 = SPEAKRS_MODEL_FILES.iter().map(|(_, size)| *size).sum();
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(900))
        .build()?;
    let mut completed = 0_u64;

    for (name, expected) in SPEAKRS_MODEL_FILES {
        let destination = model_dir.join(name);
        if tokio::fs::metadata(&destination)
            .await
            .map(|metadata| metadata.is_file() && metadata.len() == expected)
            .unwrap_or(false)
        {
            completed += expected;
            continue;
        }

        let temporary_path = model_dir.join(format!(".{name}.downloading"));
        let _ = tokio::fs::remove_file(&temporary_path).await;
        let response = client
            .get(format!(
                "https://huggingface.co/avencera/speakrs-models/resolve/{SPEAKRS_MODEL_REVISION}/{name}"
            ))
            .send()
            .await?
            .error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut output = tokio::fs::File::create(&temporary_path).await?;
        let mut downloaded = 0_u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            output.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            let _ = app.emit(
                "diarization-download-progress",
                serde_json::json!({
                    "progress": (((completed + downloaded.min(expected)) * 100 / total).min(99)) as u8,
                    "message": format!("Downloading speakrs models… ({name})"),
                    "engine": SPEAKRS_ENGINE,
                }),
            );
        }
        output.flush().await?;
        drop(output);

        if downloaded != expected {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(anyhow!(
                "speakrs model {name} downloaded {downloaded} bytes, expected {expected}"
            ));
        }
        tokio::fs::rename(&temporary_path, &destination).await?;
        completed += expected;
    }

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

/// Runs diarization to completion as part of recording finalization. Progress
/// events are observational; the returned turns are the authoritative result.
pub async fn run_diarization_task<R: Runtime>(
    app: AppHandle<R>,
    target: DiarizationTarget,
    audio_path: String,
) -> std::result::Result<Vec<SpeakerTurn>, String> {
    let settings = SETTINGS
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    if !settings.enabled {
        return Ok(Vec::new());
    }
    let engine = settings.engine;
    let collapse = settings.collapse_minor_speakers;

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

    // Decode before starting the heartbeat so it can quote the real length.
    let samples = match tokio::task::spawn_blocking(move || decode_for_diarization(&path)).await {
        Ok(Ok(samples)) => samples,
        Ok(Err(error)) => return finish_failed_diarization(&app, error.to_string()),
        Err(error) => return finish_failed_diarization(&app, error.to_string()),
    };

    let audio_seconds = samples.len() as f64 / 16_000.0;
    log::info!("Diarization: starting {engine} on {audio_seconds:.0}s of audio");
    let started = Instant::now();
    let heartbeat =
        spawn_progress_heartbeat(app.clone(), None, engine.clone(), audio_seconds);

    let selected = engine.clone();
    let result =
        tokio::task::spawn_blocking(move || diarize_with_engine(&selected, &samples, collapse))
            .await;
    drop(heartbeat);

    let elapsed = started.elapsed().as_secs_f64();
    log::info!(
        "Diarization: {engine} finished in {elapsed:.1}s ({:.1}x realtime)",
        audio_seconds / elapsed.max(0.001)
    );

    match result {
        Ok(Ok(turns)) => {
            let labels: Vec<_> = target.apply_speaker_turns(&turns).into_iter().map(|(sequence_id, speaker)| serde_json::json!({"sequenceId": sequence_id, "speaker": speaker})).collect();
            let _ = app.emit(
                "diarization-complete",
                serde_json::json!({
                    "speakers": turns.iter().map(|turn| &turn.speaker).collect::<std::collections::BTreeSet<_>>().len(),
                    "labels": labels,
                    "turns": &turns,
                }),
            );
            if let Ok(mut status) = JOB_STATUS.lock() {
                *status = DiarizationJobStatus {
                    in_progress: false,
                    message: "Speaker labels are ready".into(),
                    meeting_id: None,
                };
            }
            Ok(turns)
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
            Err(error.to_string())
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
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod collapse_tests {
    use super::{collapse_minor_speakers, SpeakerTurn, OTHERS_SPEAKER};

    fn turn(start: f64, end: f64, speaker: &str) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker: speaker.to_owned(),
        }
    }

    fn speakers(turns: &[SpeakerTurn]) -> Vec<&str> {
        let mut names: Vec<&str> = turns.iter().map(|t| t.speaker.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    #[test]
    fn merges_the_tail_of_one_phrase_speakers() {
        // Two real speakers plus three that barely register.
        let turns = vec![
            turn(0.0, 1800.0, "Speaker 1"),
            turn(1800.0, 3000.0, "Speaker 2"),
            turn(3000.0, 3002.0, "Speaker 3"),
            turn(3010.0, 3013.0, "Speaker 4"),
            turn(3020.0, 3021.0, "Speaker 5"),
        ];
        let result = collapse_minor_speakers(turns);
        assert_eq!(
            speakers(&result),
            vec!["Others", "Speaker 1", "Speaker 2"],
            "the three brief speakers should become one"
        );
    }

    #[test]
    fn keeps_a_brief_speaker_when_the_recording_is_long() {
        // 0.9% of speech, but 65s in absolute terms - a real participant.
        let turns = vec![
            turn(0.0, 3600.0, "Speaker 1"),
            turn(3600.0, 7135.0, "Speaker 2"),
            turn(7135.0, 7200.0, "Speaker 3"),
            turn(7200.0, 7205.0, "Speaker 4"),
        ];
        let result = collapse_minor_speakers(turns);
        assert!(
            result.iter().any(|t| t.speaker == "Speaker 3"),
            "65s of speech is not a one-phrase speaker: {:?}",
            speakers(&result)
        );
    }

    #[test]
    fn leaves_a_single_minor_speaker_alone() {
        // Collapsing one speaker only renames them, losing who they were.
        let turns = vec![
            turn(0.0, 600.0, "Speaker 1"),
            turn(600.0, 1200.0, "Speaker 2"),
            turn(1200.0, 1201.0, "Speaker 3"),
        ];
        let result = collapse_minor_speakers(turns);
        assert!(!speakers(&result).contains(&OTHERS_SPEAKER));
    }

    #[test]
    fn never_collapses_every_speaker() {
        // Heavily fragmented: each speaker is tiny, but the busiest must stay.
        let turns: Vec<SpeakerTurn> = (0..40)
            .map(|i| turn(i as f64, i as f64 + 0.5, &format!("Speaker {i}")))
            .collect();
        let result = collapse_minor_speakers(turns);
        let remaining = speakers(&result);
        assert!(
            remaining.iter().filter(|s| **s != OTHERS_SPEAKER).count() >= 2,
            "at least the busiest speakers must survive: {remaining:?}"
        );
    }

    #[test]
    fn joins_touching_others_turns_but_not_across_another_speaker() {
        let turns = vec![
            turn(0.0, 1800.0, "Speaker 1"),
            turn(1800.0, 3000.0, "Speaker 2"),
            // Two brief speakers back to back, then a gap with Speaker 1.
            turn(3000.0, 3002.0, "Speaker 3"),
            turn(3002.0, 3004.0, "Speaker 4"),
            turn(3004.0, 3100.0, "Speaker 1"),
            turn(3100.0, 3101.0, "Speaker 5"),
        ];
        let result = collapse_minor_speakers(turns);
        let others: Vec<&SpeakerTurn> = result
            .iter()
            .filter(|t| t.speaker == OTHERS_SPEAKER)
            .collect();
        assert_eq!(others.len(), 2, "touching turns join, separated ones do not");
        assert_eq!(others[0].start, 3000.0);
        assert_eq!(others[0].end, 3004.0);
    }

    #[test]
    fn leaves_two_speaker_output_untouched() {
        let turns = vec![
            turn(0.0, 600.0, "Speaker 1"),
            turn(600.0, 601.0, "Speaker 2"),
        ];
        let result = collapse_minor_speakers(turns.clone());
        assert_eq!(speakers(&result), speakers(&turns));
    }
}
