//! TTS settings and the speech engine shared by everything that speaks.
//! The summary reading itself is prepared in `summary_audio.rs`.

use super::qwen::{QwenTts, Variant};
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

const SETTINGS_FILE: &str = "tts-settings.json";

/// What the user chose on the settings screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSettings {
    pub enabled: bool,
    /// Which Qwen3-TTS checkpoint reads the summary.
    pub model: Variant,
    /// Read every new summary into a file as soon as it is generated, so it
    /// plays at once. Off: only when Play is pressed.
    #[serde(default = "auto_prepare_default")]
    pub auto_prepare: bool,
}

fn auto_prepare_default() -> bool {
    true
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: Variant::Small,
            auto_prepare: true,
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
    pub auto_prepare: bool,
    pub models: Vec<TtsModel>,
    /// Why the feature is unavailable, when it is.
    pub problem: Option<String>,
}

static SETTINGS: Lazy<Mutex<TtsSettings>> = Lazy::new(|| Mutex::new(TtsSettings::default()));

static ENGINE: Lazy<Mutex<Option<Arc<QwenTts>>>> = Lazy::new(|| Mutex::new(None));

pub(super) fn data_root<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf> {
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

pub(super) fn settings() -> TtsSettings {
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
        .map(|path| {
            path.join(if cfg!(windows) {
                "llama-tts.exe"
            } else {
                "llama-tts"
            })
            .is_file()
        })
        .unwrap_or(false)
    {
        Some("The speech runtime is not installed".to_string())
    } else {
        None
    };

    TtsStatus {
        enabled: settings.enabled,
        model: settings.model.id().to_string(),
        auto_prepare: settings.auto_prepare,
        models,
        problem,
    }
}

/// Store the settings.
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

/// Reading aloud is turned on and everything it needs is installed.
pub(super) fn can_speak<R: Runtime>(app: &AppHandle<R>) -> bool {
    let status = tts_get_status(app.clone());
    status.enabled && status.problem.is_none()
}

/// The checkpoint that will actually speak. The default points at the
/// smaller checkpoint, which may not be the one that is actually on disk;
/// speak with whatever is installed rather than refusing.
pub(super) fn speaking_variant<R: Runtime>(app: &AppHandle<R>) -> Variant {
    let chosen = settings().model;
    match is_installed(app, chosen) {
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
    }
}

pub(super) fn engine<R: Runtime>(app: &AppHandle<R>) -> Result<Arc<QwenTts>> {
    let wanted = speaking_variant(app);
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
