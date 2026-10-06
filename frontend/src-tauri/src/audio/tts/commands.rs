//! Tauri commands for reading a summary aloud.
//!
//! Synthesis runs chunk by chunk in the background and each finished chunk is
//! announced with a `tts-chunk` event, so playback starts after the first
//! sentence or two instead of after the whole summary.

use super::qwen::{QwenTts, Variant};
use super::text;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_store::StoreExt;

const SETTINGS_FILE: &str = "tts-settings.json";

/// What the user chose on the settings screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSettings {
    pub enabled: bool,
    /// Which Qwen3-TTS checkpoint reads the summary.
    pub model: Variant,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: Variant::Small,
        }
    }
}

/// Everything the settings screen needs, without loading the model - which
/// takes a minute once the accelerated graphs are built.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsModel {
    pub id: String,
    pub label: String,
    /// Whether this checkpoint is actually on disk.
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    pub enabled: bool,
    /// The selected checkpoint.
    pub model: String,
    pub models: Vec<TtsModel>,
    /// Why the feature is unavailable, when it is.
    pub problem: Option<String>,
}

static SETTINGS: Lazy<Mutex<TtsSettings>> = Lazy::new(|| Mutex::new(TtsSettings::default()));

static ENGINE: Lazy<Mutex<Option<Arc<QwenTts>>>> = Lazy::new(|| Mutex::new(None));
/// Id of the reading the UI is currently listening to. A background job whose
/// id no longer matches stops at the next chunk boundary, which is how both
/// "stop" and "start another reading" cancel the previous one.
static CURRENT_JOB: AtomicU64 = AtomicU64::new(0);
static NEXT_JOB: AtomicU64 = AtomicU64::new(0);

fn data_root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    crate::portable::app_data_dir(app)
        .map_err(|error| anyhow!("Cannot resolve the application data directory: {error}"))
}

/// Checkpoints live in `<app data>/models/tts/qwen/<variant>/`.
fn models_root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    Ok(data_root(app)?.join("models").join("tts").join("qwen"))
}

/// The llama.cpp binaries that run the pipeline.
fn runtime_root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    Ok(data_root(app)?.join("tts-runtime"))
}

/// The reference recording whose voice the models clone.
fn speaker_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
    Ok(data_root(app)?.join("voices").join("default.wav"))
}

fn is_installed<R: Runtime>(app: &AppHandle<R>, variant: Variant) -> bool {
    models_root(app)
        .map(|root| {
            let directory = root.join(variant.id());
            directory.join("model.gguf").is_file() && directory.join("mmproj.gguf").is_file()
        })
        .unwrap_or(false)
}

fn settings() -> TtsSettings {
    SETTINGS
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default()
}

/// Read the stored settings at startup, before anything else touches them.
pub fn load_settings<R: Runtime>(app: &AppHandle<R>) {
    let Ok(store) = app.store(crate::portable::store_path(SETTINGS_FILE)) else {
        return;
    };
    let Some(value) = store.get("settings") else {
        return;
    };
    match serde_json::from_value::<TtsSettings>(value.clone()) {
        Ok(stored) => {
            if let Ok(mut current) = SETTINGS.lock() {
                *current = stored;
            }
        }
        Err(error) => log::warn!("Ignoring unreadable TTS settings: {error}"),
    }
}

/// What the settings screen shows: which checkpoints exist on disk and which
/// one is selected. Nothing is loaded here - that costs seconds.
#[tauri::command]
pub fn tts_get_status<R: Runtime>(app: AppHandle<R>) -> TtsStatus {
    let settings = settings();
    let models: Vec<TtsModel> = Variant::all()
        .into_iter()
        .map(|variant| TtsModel {
            id: variant.id().to_string(),
            label: variant.label().to_string(),
            installed: is_installed(&app, variant),
        })
        .collect();

    let problem = if models.iter().all(|model| !model.installed) {
        Some(format!(
            "No speech model is installed in {}",
            models_root(&app)
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        ))
    } else if !runtime_root(&app)
        .map(|path| path.join(if cfg!(windows) { "llama-tts.exe" } else { "llama-tts" }).is_file())
        .unwrap_or(false)
    {
        Some("The speech runtime is not installed".to_string())
    } else {
        None
    };

    TtsStatus {
        enabled: settings.enabled,
        model: settings.model.id().to_string(),
        models,
        problem,
    }
}

/// Store the settings. Changing the voice needs no reload: it is an input to
/// the graph, not part of it.
#[tauri::command]
pub fn tts_set_settings<R: Runtime>(
    app: AppHandle<R>,
    settings: TtsSettings,
) -> Result<(), String> {
    let store = app
        .store(crate::portable::store_path(SETTINGS_FILE))
        .map_err(|error| error.to_string())?;
    store.set(
        "settings",
        serde_json::to_value(&settings).map_err(|error| error.to_string())?,
    );
    store.save().map_err(|error| error.to_string())?;
    *SETTINGS
        .lock()
        .map_err(|_| "The TTS settings lock is unavailable")? = settings;
    Ok(())
}

/// Whether the model is already in memory. `try_lock` on purpose: the lock is
/// held for the whole load, and a blocked caller is exactly the case we want
/// to report as "still loading".
fn is_loaded() -> bool {
    ENGINE
        .try_lock()
        .map(|engine| engine.is_some())
        .unwrap_or(false)
}

fn engine<R: Runtime>(app: &AppHandle<R>) -> Result<Arc<QwenTts>> {
    let chosen = settings().model;
    // The default points at the smaller checkpoint, which may not be the one
    // that is actually on disk; speak with whatever is installed rather than
    // refusing.
    let wanted = match is_installed(app, chosen) {
        true => chosen,
        false => Variant::all()
            .into_iter()
            .find(|variant| is_installed(app, *variant))
            .map(|fallback| {
                log::info!(
                    "TTS: {} is not installed, using {} instead",
                    chosen.id(),
                    fallback.id()
                );
                fallback
            })
            .unwrap_or(chosen),
    };
    let mut guard = ENGINE
        .lock()
        .map_err(|_| anyhow!("The TTS engine lock is poisoned"))?;
    if let Some(engine) = guard.as_ref() {
        if engine.variant() == wanted {
            return Ok(engine.clone());
        }
    }
    let loaded = Arc::new(QwenTts::load(
        &runtime_root(app)?,
        &models_root(app)?,
        &speaker_path(app)?,
        wanted,
    )?);
    *guard = Some(loaded.clone());
    Ok(loaded)
}

/// How long a finished WAV plays, read from its header.
fn wav_seconds(path: &Path) -> Option<f32> {
    let header = std::fs::read(path).ok()?;
    if header.len() < 44 || &header[0..4] != b"RIFF" {
        return None;
    }
    let rate = u32::from_le_bytes(header[24..28].try_into().ok()?) as f32;
    let bytes_per_sample = u16::from_le_bytes(header[34..36].try_into().ok()?) as f32 / 8.0;
    let data = (header.len() - 44) as f32;
    (rate > 0.0 && bytes_per_sample > 0.0).then(|| data / (rate * bytes_per_sample))
}

/// Synthesize the chunks of one reading, announcing each as it becomes
/// playable. Already synthesized chunks are reused, so replaying the same
/// summary starts instantly.
fn run_job<R: Runtime>(app: &AppHandle<R>, job: u64, chunks: &[String]) -> Result<()> {
    if !is_loaded() {
        let _ = app.emit(
            "tts-waiting",
            serde_json::json!({ "job": job, "message": "Preparing the speech model…" }),
        );
    }
    let engine = engine(app)?;
    // The model is part of the key: the same text read by another checkpoint
    // is different audio.
    let digest = format!(
        "{:x}",
        md5::compute(format!("{}\n{}", engine.variant().id(), chunks.join("\n")).as_bytes())
    );
    let directory = data_root(app)?.join("tts-cache").join(&digest);
    std::fs::create_dir_all(&directory)?;

    let started = std::time::Instant::now();
    let mut synthesized_seconds = 0.0_f32;
    for (index, chunk) in chunks.iter().enumerate() {
        if CURRENT_JOB.load(Ordering::SeqCst) != job {
            log::debug!("TTS job {job} cancelled after {index} chunks");
            return Ok(());
        }

        let path = directory.join(format!("{index:04}.wav"));
        if !path.is_file() {
            let chunk_started = std::time::Instant::now();
            engine.synthesize(chunk, &path)?;
            let seconds = wav_seconds(&path).unwrap_or(0.0);
            synthesized_seconds += seconds;
            log::info!(
                "TTS chunk {}/{}: {seconds:.1}s of audio in {:.1}s: {chunk}",
                index + 1,
                chunks.len(),
                chunk_started.elapsed().as_secs_f32()
            );
        }

        app.asset_protocol_scope().allow_file(&path)?;
        app.emit(
            "tts-chunk",
            serde_json::json!({
                "job": job,
                "index": index,
                "total": chunks.len(),
                "path": path.to_string_lossy(),
            }),
        )?;
    }

    log::info!(
        "TTS job {job}: {} chunks, {synthesized_seconds:.1}s synthesized in {:.1}s",
        chunks.len(),
        started.elapsed().as_secs_f32()
    );
    app.emit(
        "tts-done",
        serde_json::json!({ "job": job, "total": chunks.len() }),
    )?;
    Ok(())
}

/// Start reading `text` aloud. Returns the id of this reading; audio arrives
/// as `tts-chunk` events and the reading ends with `tts-done` or `tts-error`.
#[tauri::command]
pub async fn tts_speak<R: Runtime>(app: AppHandle<R>, text: String) -> Result<u64, String> {
    if !settings().enabled {
        return Err("Reading summaries aloud is turned off in settings".to_string());
    }
    let chunks = text::summary_to_chunks(&text);
    if chunks.is_empty() {
        return Err("There is nothing to read in this summary".to_string());
    }

    let job = NEXT_JOB.fetch_add(1, Ordering::SeqCst) + 1;
    CURRENT_JOB.store(job, Ordering::SeqCst);

    let app_for_job = app.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(error) = run_job(&app_for_job, job, &chunks) {
            log::warn!("TTS job {job} failed: {error}");
            if CURRENT_JOB.load(Ordering::SeqCst) == job {
                let _ = app_for_job.emit(
                    "tts-error",
                    serde_json::json!({ "job": job, "message": error.to_string() }),
                );
            }
        }
    });

    Ok(job)
}

/// Stop the current reading. Chunks already delivered stay on disk.
#[tauri::command]
pub fn tts_stop() {
    CURRENT_JOB.store(0, Ordering::SeqCst);
}

/// Load the model at startup, the way the transcription engines do, so the
/// first Play does not pay for reading ~1 GB of model files and building the
/// accelerated graphs. Silent when no package is installed.
pub fn preload<R: Runtime>(app: &AppHandle<R>) {
    if !settings().enabled {
        log::info!("TTS is turned off; not loading the model");
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            match engine(&app) {
                Ok(_) => log::info!(
                    "TTS engine ready in {:.1}s",
                    started.elapsed().as_secs_f32()
                ),
                Err(error) => log::info!("TTS engine not available: {error}"),
            }
        })
        .await;
    });
}
