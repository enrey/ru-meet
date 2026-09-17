use crate::audio::transcription::{TranscriptResult, TranscriptionError, TranscriptionProvider};
use crate::parakeet_engine::{DownloadProgress, ModelInfo, ModelStatus, QuantizationType};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{command, AppHandle, Emitter, Manager, Runtime};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use transcribe_rs::onnx::{gigaam::GigaAMModel, Quantization};
use transcribe_rs::{SpeechModel, TranscribeOptions};

pub const MODEL_NAME: &str = "gigaam-v3-e2e-ctc";
const MODEL_DIR: &str = "giga-am-v3-int8";
const MODEL_URL: &str =
    "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc.int8.onnx";
const VOCAB_URL: &str =
    "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc_vocab.txt";
const MODEL_SHA256: &str = "2e3fcb7a7b66030336fd10c2fcfb033bd1dc7e1bf238fe5cfd83b1d0cfc9d28e";
const VOCAB_SHA256: &str = "142de7570b3de5b3035ce111a89c228e80e6085273731d944093ddf24fa539cd";
const MODEL_SIZE_MB: u32 = 186;

pub static GIGAAM_ENGINE: Mutex<Option<Arc<GigaAmEngine>>> = Mutex::new(None);
static MODELS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub struct GigaAmEngine {
    models_dir: PathBuf,
    model: tokio::sync::Mutex<Option<GigaAMModel>>,
    downloading: AtomicBool,
}

impl GigaAmEngine {
    fn new(models_dir: PathBuf) -> Result<Self> {
        let models_dir = models_dir.join("gigaam");
        std::fs::create_dir_all(&models_dir)?;
        Ok(Self {
            models_dir,
            model: tokio::sync::Mutex::new(None),
            downloading: AtomicBool::new(false),
        })
    }

    fn model_path(&self) -> PathBuf {
        self.models_dir.join(MODEL_DIR)
    }

    fn validate_model(path: &Path) -> Result<()> {
        let onnx = path.join("model.int8.onnx");
        let fallback_onnx = path.join("model.onnx");
        if (!onnx.is_file() && !fallback_onnx.is_file()) || !path.join("vocab.txt").is_file() {
            return Err(anyhow!(
                "GigaAM model must contain model.int8.onnx (or model.onnx) and vocab.txt"
            ));
        }
        Ok(())
    }

    pub fn discover_models(&self) -> Vec<ModelInfo> {
        let path = self.model_path();
        let status = if self.downloading.load(Ordering::Acquire) {
            ModelStatus::Downloading { progress: 0 }
        } else if !path.exists() {
            ModelStatus::Missing
        } else if Self::validate_model(&path).is_ok() {
            ModelStatus::Available
        } else {
            ModelStatus::Corrupted {
                file_size: directory_size(&path),
                expected_min_size: MODEL_SIZE_MB as u64 * 1024 * 1024,
            }
        };

        vec![ModelInfo {
            name: MODEL_NAME.to_string(),
            path,
            size_mb: MODEL_SIZE_MB,
            quantization: QuantizationType::Int8,
            speed: "Fast".to_string(),
            status,
            description: "Russian speech recognition. Fast and accurate.".to_string(),
        }]
    }

    pub async fn load_model(&self, model_name: &str) -> Result<()> {
        if model_name != MODEL_NAME {
            return Err(anyhow!("Unknown GigaAM model: {model_name}"));
        }
        if self.model.lock().await.is_some() {
            return Ok(());
        }

        let path = self.model_path();
        Self::validate_model(&path)?;
        crate::ensure_onnx_runtime_available()?;
        let loaded = tokio::task::spawn_blocking(move || {
            GigaAMModel::load(&path, &Quantization::Int8).map_err(|error| error.to_string())
        })
        .await
        .context("GigaAM model load task failed")?
        .map_err(|error| anyhow!("Failed to load GigaAM model: {error}"))?;
        *self.model.lock().await = Some(loaded);
        Ok(())
    }

    pub async fn unload_model(&self) -> bool {
        self.model.lock().await.take().is_some()
    }

    pub async fn is_model_loaded(&self) -> bool {
        self.model.lock().await.is_some()
    }

    pub async fn transcribe_audio(&self, audio: Vec<f32>) -> Result<String> {
        let mut model = self.model.lock().await;
        let model = model
            .as_mut()
            .ok_or_else(|| anyhow!("No GigaAM model loaded"))?;
        model
            .transcribe(&audio, &TranscribeOptions::default())
            .map(|result| result.text)
            .map_err(|error| anyhow!("GigaAM transcription failed: {error}"))
    }

    async fn download_model<F>(&self, model_name: &str, progress: F) -> Result<()>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        if model_name != MODEL_NAME {
            return Err(anyhow!("Unknown GigaAM model: {model_name}"));
        }
        if self
            .downloading
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(anyhow!("GigaAM model download is already in progress"));
        }

        let result = self.download_model_inner(progress).await;
        self.downloading.store(false, Ordering::Release);
        result
    }

    async fn download_model_inner<F>(&self, progress: F) -> Result<()>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        fs::create_dir_all(&self.models_dir).await?;
        let download_path = self.models_dir.join(format!(".{MODEL_DIR}.downloading"));
        let final_path = self.model_path();
        if download_path.exists() {
            fs::remove_dir_all(&download_path).await?;
        }
        fs::create_dir_all(&download_path).await?;

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .build()?;
        let response = client.get(MODEL_URL).send().await?.error_for_status()?;
        let total = response
            .content_length()
            .unwrap_or(MODEL_SIZE_MB as u64 * 1024 * 1024);
        let mut stream = response.bytes_stream();
        let model_path = download_path.join("model.int8.onnx");
        let mut file = fs::File::create(&model_path).await?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0u64;
        let started = Instant::now();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;
            let speed =
                downloaded as f64 / started.elapsed().as_secs_f64().max(0.001) / 1_048_576.0;
            progress(DownloadProgress::new(downloaded, total, speed));
        }
        file.flush().await?;
        drop(file);

        let actual_hash = format!("{:x}", hasher.finalize());
        if actual_hash != MODEL_SHA256 {
            let _ = fs::remove_dir_all(&download_path).await;
            return Err(anyhow!("GigaAM model checksum mismatch"));
        }

        let vocab = client
            .get(VOCAB_URL)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if format!("{:x}", Sha256::digest(&vocab)) != VOCAB_SHA256 {
            let _ = fs::remove_dir_all(&download_path).await;
            return Err(anyhow!("GigaAM vocabulary checksum mismatch"));
        }
        fs::write(download_path.join("vocab.txt"), vocab).await?;
        Self::validate_model(&download_path)?;
        if final_path.exists() {
            fs::remove_dir_all(&final_path).await?;
        }
        fs::rename(&download_path, &final_path).await?;
        progress(DownloadProgress::new(total, total, 0.0));
        Ok(())
    }

    pub async fn delete_model(&self, model_name: &str) -> Result<()> {
        if model_name != MODEL_NAME {
            return Err(anyhow!("Unknown GigaAM model: {model_name}"));
        }
        self.unload_model().await;
        let path = self.model_path();
        if path.exists() {
            fs::remove_dir_all(path).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl TranscriptionProvider for GigaAmEngine {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        _language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        // GigaAM v3 is a Russian-only acoustic model. Unlike Whisper, it has no
        // runtime language switch: every inference is inherently Russian.
        self.transcribe_audio(audio)
            .await
            .map(|text| TranscriptResult {
                text,
                confidence: None,
                is_partial: false,
            })
            .map_err(|error| TranscriptionError::EngineFailed(error.to_string()))
    }

    async fn is_model_loaded(&self) -> bool {
        self.is_model_loaded().await
    }

    async fn get_current_model(&self) -> Option<String> {
        self.is_model_loaded().await.then(|| MODEL_NAME.to_string())
    }

    fn provider_name(&self) -> &'static str {
        "GigaAM v3"
    }
}

fn directory_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                directory_size(&path)
            } else {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

pub fn set_models_directory<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(path) = crate::portable::app_data_dir(&app) {
        *MODELS_DIR.lock().unwrap() = Some(path.join("models"));
    }
}

#[command]
pub async fn gigaam_init() -> Result<(), String> {
    let mut engine = GIGAAM_ENGINE.lock().unwrap();
    if engine.is_none() {
        let models_dir = MODELS_DIR
            .lock()
            .unwrap()
            .clone()
            .ok_or("GigaAM models directory is not initialized")?;
        *engine = Some(Arc::new(
            GigaAmEngine::new(models_dir).map_err(|e| e.to_string())?,
        ));
    }
    Ok(())
}

fn engine() -> Result<Arc<GigaAmEngine>, String> {
    GIGAAM_ENGINE
        .lock()
        .unwrap()
        .as_ref()
        .cloned()
        .ok_or_else(|| "GigaAM engine is not initialized".to_string())
}

#[command]
pub async fn gigaam_get_available_models() -> Result<Vec<ModelInfo>, String> {
    Ok(engine()?.discover_models())
}

#[command]
pub async fn gigaam_has_available_models() -> Result<bool, String> {
    Ok(engine()?
        .discover_models()
        .iter()
        .any(|model| matches!(model.status, ModelStatus::Available)))
}

#[command]
pub async fn gigaam_load_model(model_name: String) -> Result<(), String> {
    engine()?
        .load_model(&model_name)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn gigaam_is_model_loaded() -> Result<bool, String> {
    Ok(engine()?.is_model_loaded().await)
}

#[command]
pub async fn gigaam_get_current_model() -> Result<Option<String>, String> {
    Ok(engine()?
        .is_model_loaded()
        .await
        .then(|| MODEL_NAME.to_string()))
}

#[command]
pub async fn gigaam_validate_model_ready() -> Result<String, String> {
    let engine = engine()?;
    engine
        .load_model(MODEL_NAME)
        .await
        .map_err(|e| e.to_string())?;
    Ok(MODEL_NAME.to_string())
}

pub async fn gigaam_validate_model_ready_with_config<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<String, String> {
    let configured = crate::api::api::api_get_transcript_config(app.clone(), app.state(), None)
        .await
        .map_err(|e| e.to_string())?
        .filter(|config| config.provider == "gigaam" && !config.model.is_empty())
        .map(|config| config.model)
        .unwrap_or_else(|| MODEL_NAME.to_string());
    let engine = engine()?;
    engine
        .load_model(&configured)
        .await
        .map_err(|e| e.to_string())?;
    Ok(configured)
}

#[command]
pub async fn gigaam_transcribe_audio(audio_data: Vec<f32>) -> Result<String, String> {
    engine()?
        .transcribe_audio(audio_data)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn gigaam_download_model<R: Runtime>(
    app: AppHandle<R>,
    model_name: String,
) -> Result<(), String> {
    let engine = engine()?;
    let progress_app = app.clone();
    let progress_model = model_name.clone();
    let result = engine
        .download_model(&model_name, move |progress| {
            let _ = progress_app.emit(
                "gigaam-model-download-progress",
                serde_json::json!({
                    "modelName": progress_model,
                    "progress": progress.percent,
                    "downloaded_mb": progress.downloaded_mb,
                    "total_mb": progress.total_mb,
                    "speed_mbps": progress.speed_mbps,
                    "status": if progress.percent == 100 { "completed" } else { "downloading" }
                }),
            );
        })
        .await;

    match result {
        Ok(()) => {
            let _ = app.emit(
                "gigaam-model-download-complete",
                serde_json::json!({ "modelName": model_name }),
            );
            crate::tray::update_tray_menu(&app);
            Ok(())
        }
        Err(error) => {
            let _ = app.emit(
                "gigaam-model-download-error",
                serde_json::json!({ "modelName": model_name, "error": error.to_string() }),
            );
            Err(error.to_string())
        }
    }
}

#[command]
pub async fn gigaam_delete_model(model_name: String) -> Result<(), String> {
    engine()?
        .delete_model(&model_name)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn gigaam_get_models_directory() -> Result<String, String> {
    Ok(engine()?.models_dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_required_gigaam_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.int8.onnx"), []).unwrap();
        assert!(GigaAmEngine::validate_model(dir.path()).is_err());
        std::fs::write(dir.path().join("vocab.txt"), []).unwrap();
        assert!(GigaAmEngine::validate_model(dir.path()).is_ok());
    }
}
