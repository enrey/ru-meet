use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::database::repositories::setting::SettingsRepository;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnboardingStatus {
    pub version: String,
    pub completed: bool,
    pub current_step: u8,
    pub model_status: ModelStatus,
    #[serde(default = "default_transcription_provider")]
    pub transcription_provider: String,
    #[serde(default = "default_true")]
    pub download_transcription: bool,
    #[serde(default)]
    pub download_summary: bool,
    #[serde(default)]
    pub download_diarization: bool,
    #[serde(default = "default_diarization_engine")]
    pub diarization_engine: String,
    pub last_updated: String,
}

fn default_transcription_provider() -> String {
    "gigaam".to_string()
}
fn default_true() -> bool {
    true
}
fn default_diarization_engine() -> String {
    "pyannote-wespeaker".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ModelStatus {
    pub parakeet: String, // "downloaded" | "not_downloaded" | "downloading"
    pub summary: String,  // Generic field for summary model (Qwen 3.5 or legacy Gemma variants)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_summary_model: Option<String>,
}

impl Default for OnboardingStatus {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            completed: false,
            current_step: 1,
            transcription_provider: default_transcription_provider(),
            download_transcription: true,
            download_summary: false,
            download_diarization: false,
            diarization_engine: default_diarization_engine(),
            model_status: ModelStatus {
                parakeet: "not_downloaded".to_string(),
                summary: "not_downloaded".to_string(), // Changed from gemma
                selected_summary_model: None,
            },
            last_updated: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Load onboarding status from store
pub async fn load_onboarding_status<R: Runtime>(app: &AppHandle<R>) -> Result<OnboardingStatus> {
    // Try to load from Tauri store
    let store = match app.store(crate::portable::store_path("onboarding-status.json")) {
        Ok(store) => store,
        Err(e) => {
            warn!("Failed to access onboarding store: {}, using defaults", e);
            return Ok(OnboardingStatus::default());
        }
    };

    // Try to get the status from store
    let status = if let Some(value) = store.get("status") {
        match serde_json::from_value::<OnboardingStatus>(value.clone()) {
            Ok(s) => {
                info!(
                    "Loaded onboarding status from store - Step: {}, Completed: {}",
                    s.current_step, s.completed
                );
                s
            }
            Err(e) => {
                warn!(
                    "Failed to deserialize onboarding status: {}, using defaults",
                    e
                );
                OnboardingStatus::default()
            }
        }
    } else {
        info!("No stored onboarding status found, using defaults");
        OnboardingStatus::default()
    };

    Ok(status)
}

/// Save onboarding status to store
pub async fn save_onboarding_status<R: Runtime>(
    app: &AppHandle<R>,
    status: &OnboardingStatus,
) -> Result<()> {
    info!(
        "Saving onboarding status: step={}, completed={}",
        status.current_step, status.completed
    );

    // Get or create store
    let store = app
        .store(crate::portable::store_path("onboarding-status.json"))
        .map_err(|e| anyhow::anyhow!("Failed to access onboarding store: {}", e))?;

    // Update last_updated timestamp
    let mut status = status.clone();
    status.last_updated = chrono::Utc::now().to_rfc3339();

    // Serialize status to JSON value
    let status_value = serde_json::to_value(&status)
        .map_err(|e| anyhow::anyhow!("Failed to serialize onboarding status: {}", e))?;

    // Save to store
    store.set("status", status_value);

    // Persist to disk
    store
        .save()
        .map_err(|e| anyhow::anyhow!("Failed to save onboarding store to disk: {}", e))?;

    info!("Successfully persisted onboarding status to disk");
    Ok(())
}

/// Reset onboarding status (delete from store)
pub async fn reset_onboarding_status<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    info!("Resetting onboarding status");

    let store = app
        .store(crate::portable::store_path("onboarding-status.json"))
        .map_err(|e| anyhow::anyhow!("Failed to access onboarding store: {}", e))?;

    // Clear the status key
    store.delete("status");

    // Persist deletion to disk
    store
        .save()
        .map_err(|e| anyhow::anyhow!("Failed to save onboarding store after reset: {}", e))?;

    info!("Successfully reset onboarding status");
    Ok(())
}

/// Tauri commands for onboarding status
#[tauri::command]
pub async fn get_onboarding_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<OnboardingStatus>, String> {
    let status = load_onboarding_status(&app)
        .await
        .map_err(|e| format!("Failed to load onboarding status: {}", e))?;

    // Return None if it's the default (never saved before)
    // Check if we have any saved data by seeing if the store has the key
    let store = app
        .store(crate::portable::store_path("onboarding-status.json"))
        .map_err(|e| format!("Failed to access store: {}", e))?;

    if store.get("status").is_none() {
        Ok(None)
    } else {
        Ok(Some(status))
    }
}

#[tauri::command]
pub async fn save_onboarding_status_cmd<R: Runtime>(
    app: AppHandle<R>,
    status: OnboardingStatus,
) -> Result<(), String> {
    save_onboarding_status(&app, &status)
        .await
        .map_err(|e| format!("Failed to save onboarding status: {}", e))
}

#[tauri::command]
pub async fn reset_onboarding_status_cmd<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    reset_onboarding_status(&app)
        .await
        .map_err(|e| format!("Failed to reset onboarding status: {}", e))
}

#[tauri::command]
pub async fn complete_onboarding<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    model: Option<String>,
    transcription_provider: String,
    download_transcription: bool,
    download_summary: bool,
    download_diarization: bool,
    diarization_engine: String,
) -> Result<(), String> {
    if transcription_provider != "gigaam" && transcription_provider != "parakeet" {
        return Err("Unsupported transcription provider".to_string());
    }
    crate::audio::diarization::set_diarization_settings(
        app.clone(),
        crate::audio::diarization::DiarizationSettings {
            enabled: download_diarization,
            engine: diarization_engine.clone(),
        },
    )?;
    info!(
        "Completing onboarding with transcription provider: {}",
        transcription_provider
    );

    // Step 1: Save model configuration to SQLite database FIRST
    let pool = state.db_manager.pool();

    // Onboarding always uses builtin-ai (local LLM)
    if let Some(ref model) = model {
        SettingsRepository::save_model_config(pool, "builtin-ai", model, "large-v3", None)
            .await
            .map_err(|e| format!("Failed to save builtin-ai model config: {}", e))?;
    }

    // Save transcription model config (parakeet provider) - always parakeet
    if let Err(e) = SettingsRepository::save_transcript_config(
        pool,
        &transcription_provider,
        if transcription_provider == "gigaam" {
            "gigaam-v3-e2e-ctc"
        } else {
            crate::config::DEFAULT_PARAKEET_MODEL
        },
    )
    .await
    {
        error!("Failed to save transcription model config: {}", e);
        return Err(format!("Failed to save transcription model config: {}", e));
    }
    info!(
        "Saved transcription model config: provider={}",
        transcription_provider
    );

    // Step 2: Only NOW mark onboarding as complete (after DB operations succeed)
    let mut status = load_onboarding_status(&app)
        .await
        .map_err(|e| format!("Failed to load onboarding status: {}", e))?;

    status.completed = true;
    status.current_step = 4; // Max step (4 on macOS with permissions, 3 on other platforms)
    status.transcription_provider = transcription_provider;
    status.download_transcription = download_transcription;
    status.download_summary = download_summary;
    status.download_diarization = download_diarization;
    status.diarization_engine = diarization_engine;
    status.model_status.selected_summary_model = model.clone();

    save_onboarding_status(&app, &status)
        .await
        .map_err(|e| format!("Failed to save completed onboarding status: {}", e))?;

    info!("Onboarding completed successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_status_deserializes_without_selected_summary_model() {
        let status: OnboardingStatus = serde_json::from_str(
            r#"{
                "version": "1.0",
                "completed": true,
                "current_step": 4,
                "model_status": {
                    "parakeet": "downloaded",
                    "summary": "downloaded"
                },
                "last_updated": "2026-05-30T00:00:00Z"
            }"#,
        )
        .expect("old onboarding status should remain compatible");

        assert_eq!(status.model_status.selected_summary_model, None);
    }
}
