// audio/transcription/engine.rs
//
// TranscriptionEngine enum and model initialization/validation logic.

use super::parakeet_provider::ParakeetProvider;
use super::provider::{TranscriptResult, TranscriptionError, TranscriptionProvider};
use super::whisper_provider::WhisperProvider;
use log::{info, warn};
use once_cell::sync::Lazy;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::Mutex as AsyncMutex;

// ============================================================================
// TRANSCRIPTION ENGINE INTERFACE
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderId {
    Whisper,
    Parakeet,
    GigaAm,
}

impl ProviderId {
    pub fn parse(value: Option<&str>, default: Self) -> Result<Self, String> {
        match value {
            None => Ok(default),
            Some("localWhisper" | "whisper") => Ok(Self::Whisper),
            Some("parakeet") => Ok(Self::Parakeet),
            Some("gigaam") => Ok(Self::GigaAm),
            Some(other) => Err(format!("Unsupported transcription provider: {other}")),
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Self::Whisper => crate::config::DEFAULT_WHISPER_MODEL,
            Self::Parakeet => crate::config::DEFAULT_PARAKEET_MODEL,
            Self::GigaAm => crate::config::DEFAULT_GIGAAM_MODEL,
        }
    }
}

#[derive(Clone)]
pub struct TranscriptionEngine {
    provider_id: ProviderId,
    provider: Arc<dyn TranscriptionProvider>,
}

impl TranscriptionEngine {
    fn new(provider_id: ProviderId, provider: Arc<dyn TranscriptionProvider>) -> Self {
        Self {
            provider_id,
            provider,
        }
    }

    pub async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> Result<TranscriptResult, TranscriptionError> {
        self.provider.transcribe(audio, language).await
    }

    /// Check if the engine has a model loaded
    pub async fn is_model_loaded(&self) -> bool {
        self.provider.is_model_loaded().await
    }

    /// Get the current model name
    pub async fn get_current_model(&self) -> Option<String> {
        self.provider.get_current_model().await
    }

    /// Get the provider name for logging
    pub fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }

    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }
}

static ENGINE_LIFECYCLE_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));

// ============================================================================
// MODEL VALIDATION AND INITIALIZATION
// ============================================================================

/// Transcript configuration the live recording uses, falling back to the
/// local default when none is stored or it cannot be read.
async fn recording_transcript_config<R: Runtime>(
    app: &AppHandle<R>,
) -> crate::api::api::TranscriptConfig {
    match crate::api::api::api_get_transcript_config(app.clone(), app.clone().state(), None).await
    {
        Ok(Some(config)) => {
            info!(
                "📝 Transcript config - provider: {}, model: {}",
                config.provider, config.model
            );
            config
        }
        Ok(None) => {
            info!("📝 No transcript config found, using the local default");
            crate::api::api::TranscriptConfig::local_default()
        }
        Err(e) => {
            warn!(
                "⚠️ Failed to get transcript config: {}, using the local default",
                e
            );
            crate::api::api::TranscriptConfig::local_default()
        }
    }
}

/// Cheap pre-start check: the model the recording will transcribe with is on
/// disk. It does not load the model — the transcription task does that in the
/// background, so audio capture starts immediately and speech segments queue
/// until the model is ready.
pub async fn ensure_transcription_model_available<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let config = recording_transcript_config(app).await;
    let provider = ProviderId::parse(Some(&config.provider), ProviderId::GigaAm)?;
    let model = resolve_model(app, provider, Some(&config.model)).await?;

    let available = match provider {
        ProviderId::Whisper => {
            crate::whisper_engine::commands::whisper_init().await?;
            let engine = crate::whisper_engine::commands::WHISPER_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| "Whisper engine not initialized".to_string())?;
            engine
                .discover_models()
                .await
                .map_err(|error| format!("Failed to discover Whisper models: {error}"))?
                .iter()
                .any(|info| {
                    info.name == model
                        && matches!(info.status, crate::whisper_engine::ModelStatus::Available)
                })
        }
        ProviderId::Parakeet => {
            crate::parakeet_engine::commands::parakeet_init().await?;
            let engine = crate::parakeet_engine::commands::PARAKEET_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| "Parakeet engine not initialized".to_string())?;
            engine
                .discover_models()
                .await
                .map_err(|error| format!("Failed to discover Parakeet models: {error}"))?
                .iter()
                .any(|info| {
                    info.name == model
                        && matches!(info.status, crate::parakeet_engine::ModelStatus::Available)
                })
        }
        ProviderId::GigaAm => {
            crate::gigaam_engine::gigaam_init().await?;
            let engine = crate::gigaam_engine::GIGAAM_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| "GigaAM engine not initialized".to_string())?;
            engine.discover_models().iter().any(|info| {
                info.name == model
                    && matches!(info.status, crate::parakeet_engine::ModelStatus::Available)
            })
        }
    };

    if available {
        info!("✅ Transcription model '{}' is on disk; it loads in the background", model);
        Ok(())
    } else {
        Err(format!(
            "Speech recognition model '{model}' is not downloaded. Download it in Settings › Transcription."
        ))
    }
}

/// Get or initialize the appropriate transcription engine based on provider configuration
pub async fn get_or_init_transcription_engine<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<TranscriptionEngine, String> {
    let config = recording_transcript_config(app).await;
    get_or_init_batch_engine(app, Some(&config.provider), Some(&config.model)).await
}

/// Acquire the engine requested by an import or retranscription job.
/// Batch callers default to Whisper for backward compatibility.
pub async fn get_or_init_batch_engine<R: Runtime>(
    app: &AppHandle<R>,
    provider: Option<&str>,
    requested_model: Option<&str>,
) -> Result<TranscriptionEngine, String> {
    let _guard = ENGINE_LIFECYCLE_LOCK.lock().await;
    let provider_id = ProviderId::parse(provider, ProviderId::Whisper)?;
    let model = resolve_model(app, provider_id, requested_model).await?;

    match provider_id {
        ProviderId::Whisper => {
            crate::whisper_engine::commands::whisper_init().await?;
            let engine = crate::whisper_engine::commands::WHISPER_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| "Whisper engine not initialized".to_string())?;
            ensure_whisper_model(&engine, &model).await?;
            Ok(TranscriptionEngine::new(
                provider_id,
                Arc::new(WhisperProvider::new(engine)),
            ))
        }
        ProviderId::Parakeet => {
            crate::parakeet_engine::commands::parakeet_init().await?;
            let engine = crate::parakeet_engine::commands::PARAKEET_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| "Parakeet engine not initialized".to_string())?;
            ensure_parakeet_model(&engine, &model).await?;
            Ok(TranscriptionEngine::new(
                provider_id,
                Arc::new(ParakeetProvider::new(engine)),
            ))
        }
        ProviderId::GigaAm => {
            crate::gigaam_engine::gigaam_init().await?;
            let engine = crate::gigaam_engine::GIGAAM_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned()
                .ok_or_else(|| "GigaAM engine not initialized".to_string())?;
            if engine.get_current_model().await.as_deref() != Some(model.as_str()) {
                engine
                    .load_model(&model)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(TranscriptionEngine::new(provider_id, engine))
        }
    }
}

/// Unload the batch provider unless a live recording is using the shared engine.
pub async fn unload_batch_engine(provider: Option<&str>) {
    let _guard = ENGINE_LIFECYCLE_LOCK.lock().await;
    if crate::audio::recording_commands::is_recording().await {
        info!("Skipping model unload after batch: recording in progress");
        return;
    }

    if let Ok(provider_id) = ProviderId::parse(provider, ProviderId::Whisper) {
        unload_provider(provider_id).await;
    }
}

/// Unload the configured recording provider. Provider details stay inside this module.
pub async fn unload_configured_engine<R: Runtime>(app: &AppHandle<R>) {
    let local_default = crate::api::api::TranscriptConfig::local_default();
    let provider =
        crate::api::api::api_get_transcript_config(app.clone(), app.clone().state(), None)
            .await
            .ok()
            .flatten()
            .map(|config| config.provider);
    let default_provider = ProviderId::parse(Some(&local_default.provider), ProviderId::GigaAm)
        .unwrap_or(ProviderId::GigaAm);
    let provider_id =
        ProviderId::parse(provider.as_deref(), default_provider).unwrap_or(default_provider);
    let _guard = ENGINE_LIFECYCLE_LOCK.lock().await;
    unload_provider(provider_id).await;
}

async fn resolve_model<R: Runtime>(
    app: &AppHandle<R>,
    provider: ProviderId,
    requested_model: Option<&str>,
) -> Result<String, String> {
    if let Some(model) = requested_model.filter(|model| !model.is_empty()) {
        return Ok(model.to_string());
    }

    let configured =
        crate::api::api::api_get_transcript_config(app.clone(), app.clone().state(), None)
            .await
            .map_err(|error| error.to_string())?;
    if let Some(config) = configured {
        if ProviderId::parse(Some(&config.provider), provider).ok() == Some(provider)
            && !config.model.is_empty()
        {
            return Ok(config.model);
        }
    }
    Ok(provider.default_model().to_string())
}

async fn ensure_whisper_model(
    engine: &Arc<crate::whisper_engine::WhisperEngine>,
    model: &str,
) -> Result<(), String> {
    if engine.get_current_model().await.as_deref() == Some(model) {
        return Ok(());
    }
    if let Err(error) = engine.discover_models().await {
        warn!("Whisper model discovery failed before load: {error}");
    }
    engine
        .load_model(model)
        .await
        .map_err(|error| format!("Failed to load Whisper model '{model}': {error}"))
}

async fn ensure_parakeet_model(
    engine: &Arc<crate::parakeet_engine::ParakeetEngine>,
    model: &str,
) -> Result<(), String> {
    if engine.get_current_model().await.as_deref() == Some(model) {
        return Ok(());
    }
    if let Err(error) = engine.discover_models().await {
        warn!("Parakeet model discovery failed before load: {error}");
    }
    engine
        .load_model(model)
        .await
        .map_err(|error| format!("Failed to load Parakeet model '{model}': {error}"))
}

async fn unload_provider(provider: ProviderId) {
    match provider {
        ProviderId::Whisper => {
            let engine = crate::whisper_engine::commands::WHISPER_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned();
            if let Some(engine) = engine {
                let _ = engine.unload_model().await;
            }
        }
        ProviderId::Parakeet => {
            let engine = crate::parakeet_engine::commands::PARAKEET_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned();
            if let Some(engine) = engine {
                let _ = engine.unload_model().await;
            }
        }
        ProviderId::GigaAm => {
            let engine = crate::gigaam_engine::GIGAAM_ENGINE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
                .cloned();
            if let Some(engine) = engine {
                let _ = engine.unload_model().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct FakeProvider;

    #[async_trait]
    impl TranscriptionProvider for FakeProvider {
        async fn transcribe(
            &self,
            audio: Vec<f32>,
            _language: Option<String>,
        ) -> Result<TranscriptResult, TranscriptionError> {
            Ok(TranscriptResult {
                text: format!("{} samples", audio.len()),
                confidence: Some(0.75),
                is_partial: false,
            })
        }

        async fn is_model_loaded(&self) -> bool {
            true
        }

        async fn get_current_model(&self) -> Option<String> {
            Some("fake".to_string())
        }

        fn provider_name(&self) -> &'static str {
            "Fake"
        }
    }

    #[test]
    fn provider_ids_are_canonicalized_in_one_place() {
        assert_eq!(
            ProviderId::parse(Some("localWhisper"), ProviderId::GigaAm).unwrap(),
            ProviderId::Whisper
        );
        assert_eq!(
            ProviderId::parse(Some("whisper"), ProviderId::GigaAm).unwrap(),
            ProviderId::Whisper
        );
        assert_eq!(
            ProviderId::parse(None, ProviderId::GigaAm).unwrap(),
            ProviderId::GigaAm
        );
        assert!(ProviderId::parse(Some("unknown"), ProviderId::GigaAm).is_err());
    }

    #[tokio::test]
    async fn engine_exposes_only_the_provider_port() {
        let engine = TranscriptionEngine::new(ProviderId::Whisper, Arc::new(FakeProvider));
        let result = engine
            .transcribe(vec![0.0; 1600], Some("ru".into()))
            .await
            .unwrap();

        assert_eq!(engine.provider_name(), "Fake");
        assert_eq!(engine.provider_id(), ProviderId::Whisper);
        assert_eq!(result.text, "1600 samples");
        assert_eq!(result.confidence, Some(0.75));
    }
}
