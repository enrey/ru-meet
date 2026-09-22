use std::sync::{atomic::Ordering, Arc, LazyLock, Mutex, MutexGuard};

use log::{error, info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::task::JoinHandle;

use super::{
    recording_commands::{self, FinalizedRecording},
    recording_manager::RecordingStartError,
    recording_state::RecordingState,
    transcription::{self, reset_speech_detected_flag},
    RecordingManager,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Starting,
    Recording,
    Paused,
    Stopping,
}

struct SessionData {
    phase: SessionPhase,
    generation: u64,
    manager: Option<RecordingManager>,
    transcription_task: Option<JoinHandle<()>>,
    device_recovery_task: Option<JoinHandle<()>>,
}

impl Default for SessionData {
    fn default() -> Self {
        Self {
            phase: SessionPhase::Idle,
            generation: 0,
            manager: None,
            transcription_task: None,
            device_recovery_task: None,
        }
    }
}

/// The single authority for an active recording and its lifecycle resources.
/// Commands may request transitions, but cannot mutate lifecycle state directly.
pub struct RecordingSession {
    data: Mutex<SessionData>,
}

pub struct StopResources {
    pub generation: u64,
    pub manager: RecordingManager,
    pub transcription_task: Option<JoinHandle<()>>,
    pub device_recovery_task: Option<JoinHandle<()>>,
}

struct StartGuard<'a>(&'a RecordingSession, u64);

impl Drop for StartGuard<'_> {
    fn drop(&mut self) {
        self.0.abort_start(self.1);
    }
}

struct StopGuard<'a>(&'a RecordingSession, u64);

impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        self.0.finish_stop(self.1);
    }
}

impl RecordingSession {
    fn new() -> Self {
        Self {
            data: Mutex::new(SessionData::default()),
        }
    }

    pub async fn start<R: Runtime>(
        &self,
        app: AppHandle<R>,
        microphone: Option<String>,
        system_audio: Option<String>,
        meeting_name: Option<String>,
    ) -> Result<(), String> {
        info!(
            "Starting recording with specific devices: mic={:?}, system={:?}, meeting={:?}",
            microphone, system_audio, meeting_name
        );

        let generation = self.begin_start()?;
        let _start_guard = StartGuard(self, generation);
        if let Err(error) = crate::ensure_onnx_runtime_available() {
            return Err(recording_commands::map_recording_start_error(
                &app,
                RecordingStartError::TranscriptionRuntime(error),
            ));
        }

        info!("Validating transcription model availability before starting recording...");
        if let Err(validation_error) = transcription::validate_transcription_model_ready(&app).await
        {
            error!("Model validation failed: {}", validation_error);
            let _ = app.emit(
                "transcription-error",
                serde_json::json!({
                    "error": validation_error,
                    "userMessage": format!("Recording cannot start: {}", validation_error),
                    "actionable": false,
                    "phase": "startup"
                }),
            );
            return Err(validation_error);
        }

        let _ = app.emit(
            "recording-starting",
            serde_json::json!({"message": "Recording initialization started"}),
        );

        let preferences = super::recording_preferences::load_recording_preferences(&app)
            .await
            .map_err(|error| {
                warn!("Failed to load recording preferences, using defaults: {error}");
                error
            })
            .ok();
        let preferred_mic = microphone.as_deref().or_else(|| {
            preferences
                .as_ref()
                .and_then(|preferences| preferences.preferred_mic_device.as_deref())
        });
        let preferred_system = system_audio.as_deref().or_else(|| {
            preferences
                .as_ref()
                .and_then(|preferences| preferences.preferred_system_device.as_deref())
        });

        #[cfg(not(target_os = "macos"))]
        let mic_device = recording_commands::resolve_mic_or_default(&app, preferred_mic);
        let system_device = recording_commands::resolve_system_or_default(preferred_system);
        #[cfg(target_os = "macos")]
        recording_commands::prepare_audio_for_recording(system_device.as_deref()).await?;
        #[cfg(target_os = "macos")]
        let mic_device = recording_commands::resolve_mic_or_default(&app, preferred_mic);

        let mut manager = RecordingManager::new();
        let auto_save = preferences
            .as_ref()
            .map_or(true, |preferences| preferences.auto_save);
        let effective_meeting_name = meeting_name.unwrap_or_else(|| {
            let now = chrono::Local::now();
            format!("Meeting {}", now.format("%Y-%m-%d_%H-%M-%S"))
        });
        manager.set_meeting_name(Some(effective_meeting_name));

        let app_for_error = app.clone();
        manager.set_error_callback(move |error| {
            let _ = app_for_error.emit("recording-error", error.user_message());
        });

        let transcription_receiver = manager
            .start_recording(mic_device, system_device, auto_save)
            .await
            .map_err(|error| recording_commands::map_recording_start_error(&app, error))?;
        let device_event_receiver = manager.take_device_event_receiver();
        let state = manager.get_state().clone();
        let transcript_target = manager.transcript_target();
        self.activate(generation, manager)?;

        if let Some(receiver) = device_event_receiver {
            let task =
                recording_commands::spawn_device_event_processor(app.clone(), receiver, state);
            self.install_device_recovery_task(generation, task)?;
        }

        recording_commands::MIC_FALLBACK_FAILED_ATTEMPTS.store(0, Ordering::SeqCst);
        reset_speech_detected_flag();
        let task = transcription::start_transcription_task(
            app.clone(),
            transcription_receiver,
            transcript_target,
        );
        self.install_transcription_task(generation, task)?;

        let _ = app.emit(
            "recording-started",
            serde_json::json!({
                "message": "Recording started with custom devices and parallel processing",
                "devices": [
                    microphone.unwrap_or_else(|| "Default Microphone".to_string()),
                    system_audio.unwrap_or_else(|| "Default System Audio".to_string())
                ],
                "workers": 3
            }),
        );
        crate::tray::update_tray_menu(&app);
        info!("Recording started with custom devices using async-first approach");
        Ok(())
    }

    pub async fn stop<R: Runtime>(
        &self,
        app: AppHandle<R>,
    ) -> Result<Option<FinalizedRecording>, String> {
        let Some(mut resources) = self.begin_stop()? else {
            return Ok(None);
        };
        let _stop_guard = StopGuard(self, resources.generation);

        let _ = app.emit("recording-shutdown-progress", serde_json::json!({"stage":"stopping_audio", "message":"Stopping audio capture...", "progress":20}));
        resources
            .manager
            .stop_streams_and_force_flush()
            .await
            .map_err(|error| format!("Failed to stop audio streams: {error}"))?;
        if let Some(task) = resources.device_recovery_task.take() {
            task.abort();
            let _ = task.await;
        }

        let _ = app.emit("recording-shutdown-progress", serde_json::json!({"stage":"processing_transcripts", "message":"Processing remaining transcript chunks...", "progress":45}));
        if let Some(task) = resources.transcription_task.take() {
            task.await
                .map_err(|error| format!("Transcription worker failed: {error}"))?;
        }
        recording_commands::unload_transcription_engine(&app).await;

        let meeting_name = resources
            .manager
            .get_meeting_name()
            .unwrap_or_else(|| "Meeting".to_string());
        let folder_path = resources
            .manager
            .get_meeting_folder()
            .map(|path| path.to_string_lossy().to_string());
        let diarization_target = resources.manager.diarization_target();
        let _ = app.emit("recording-shutdown-progress", serde_json::json!({"stage":"finalizing", "message":"Finalizing recording...", "progress":70}));
        let audio_path = resources
            .manager
            .save_recording_only(&app)
            .await
            .map_err(|error| format!("Failed to save recording files: {error}"))?;

        let (turns, diarization_status) = if let Some(audio_path) = audio_path {
            match super::diarization::run_diarization_task(
                app.clone(),
                diarization_target,
                audio_path,
            )
            .await
            {
                Ok(turns) if turns.is_empty() => (turns, "skipped".to_string()),
                Ok(turns) => (turns, "completed".to_string()),
                Err(error) => {
                    warn!("Diarization reached a failed terminal state: {error}");
                    (Vec::new(), "failed".to_string())
                }
            }
        } else {
            (Vec::new(), "skipped".to_string())
        };

        let segments = resources.manager.get_transcript_segments();
        let database_segments: Vec<crate::api::TranscriptSegment> = segments
            .iter()
            .map(|segment| crate::api::TranscriptSegment {
                id: segment.id.clone(),
                text: segment.text.clone(),
                timestamp: segment.display_time.clone(),
                audio_start_time: Some(segment.audio_start_time),
                audio_end_time: Some(segment.audio_end_time),
                duration: Some(segment.duration),
                speaker: segment.speaker.clone(),
            })
            .collect();
        let app_state = app
            .try_state::<crate::state::AppState>()
            .ok_or_else(|| "Database is unavailable; recording files were preserved".to_string())?;
        let meeting_id =
            crate::database::repositories::transcript::TranscriptsRepository::save_transcript(
                app_state.db_manager.pool(),
                &meeting_name,
                &database_segments,
                folder_path.clone(),
            )
            .await
            .map_err(|error| format!("Failed to persist finalized recording: {error}"))?;
        if !turns.is_empty() {
            crate::database::repositories::transcript::TranscriptsRepository::apply_speaker_turns(
                app_state.db_manager.pool(),
                &meeting_id,
                &turns,
            )
            .await
            .map_err(|error| format!("Failed to persist speaker labels: {error}"))?;
        }

        let result = FinalizedRecording {
            meeting_id,
            meeting_name,
            folder_path,
            transcript_count: segments.len(),
            diarization_status,
        };
        let _ = app.emit("recording-shutdown-progress", serde_json::json!({"stage":"complete", "message":"Recording stopped successfully", "progress":100}));
        let _ = app.emit("recording-stopped", &result);
        crate::tray::update_tray_menu(&app);
        Ok(Some(result))
    }

    fn lock(&self) -> MutexGuard<'_, SessionData> {
        self.data.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn begin_start(&self) -> Result<u64, String> {
        let mut data = self.lock();
        if data.phase != SessionPhase::Idle {
            return Err(format!(
                "Cannot start recording while session is {:?}",
                data.phase
            ));
        }
        data.generation = data.generation.wrapping_add(1);
        data.phase = SessionPhase::Starting;
        Ok(data.generation)
    }

    pub fn activate(&self, generation: u64, manager: RecordingManager) -> Result<(), String> {
        let mut data = self.lock();
        if data.generation != generation || data.phase != SessionPhase::Starting {
            return Err("Recording start belongs to a stale session".into());
        }
        data.manager = Some(manager);
        data.phase = SessionPhase::Recording;
        Ok(())
    }

    pub fn install_transcription_task(
        &self,
        generation: u64,
        task: JoinHandle<()>,
    ) -> Result<(), String> {
        let mut data = self.lock();
        if data.generation != generation || data.phase != SessionPhase::Recording {
            task.abort();
            return Err("Transcription task belongs to a stale session".into());
        }
        data.transcription_task = Some(task);
        Ok(())
    }

    pub fn install_device_recovery_task(
        &self,
        generation: u64,
        task: JoinHandle<()>,
    ) -> Result<(), String> {
        let mut data = self.lock();
        if data.generation != generation || data.phase != SessionPhase::Recording {
            task.abort();
            return Err("Device recovery task belongs to a stale session".into());
        }
        data.device_recovery_task = Some(task);
        Ok(())
    }

    pub fn abort_start(&self, generation: u64) {
        let mut data = self.lock();
        if data.generation == generation && data.phase == SessionPhase::Starting {
            data.manager = None;
            data.transcription_task = None;
            data.device_recovery_task = None;
            data.phase = SessionPhase::Idle;
        }
    }

    pub fn begin_stop(&self) -> Result<Option<StopResources>, String> {
        let mut data = self.lock();
        match data.phase {
            SessionPhase::Idle => return Ok(None),
            SessionPhase::Starting => return Err("Recording is still starting".into()),
            SessionPhase::Stopping => return Err("Recording is already stopping".into()),
            SessionPhase::Recording | SessionPhase::Paused => {}
        }
        data.phase = SessionPhase::Stopping;
        let manager = data
            .manager
            .take()
            .ok_or_else(|| "Active recording has no manager".to_string())?;
        Ok(Some(StopResources {
            generation: data.generation,
            manager,
            transcription_task: data.transcription_task.take(),
            device_recovery_task: data.device_recovery_task.take(),
        }))
    }

    pub fn finish_stop(&self, generation: u64) {
        let mut data = self.lock();
        if data.generation == generation && data.phase == SessionPhase::Stopping {
            data.manager = None;
            data.transcription_task = None;
            data.device_recovery_task = None;
            data.phase = SessionPhase::Idle;
        }
    }

    pub fn phase(&self) -> SessionPhase {
        self.lock().phase
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.phase(),
            SessionPhase::Starting
                | SessionPhase::Recording
                | SessionPhase::Paused
                | SessionPhase::Stopping
        )
    }

    pub fn is_live_for_state(&self, state: &Arc<RecordingState>) -> bool {
        let data = self.lock();
        matches!(data.phase, SessionPhase::Recording | SessionPhase::Paused)
            && data
                .manager
                .as_ref()
                .is_some_and(|manager| Arc::ptr_eq(manager.get_state(), state))
    }

    pub fn with_manager<T>(
        &self,
        operation: impl FnOnce(&RecordingManager) -> T,
    ) -> Result<T, String> {
        let data = self.lock();
        let manager = data
            .manager
            .as_ref()
            .ok_or_else(|| "No active recording".to_string())?;
        Ok(operation(manager))
    }

    pub fn with_manager_mut<T>(
        &self,
        operation: impl FnOnce(&mut RecordingManager) -> T,
    ) -> Result<T, String> {
        let mut data = self.lock();
        let manager = data
            .manager
            .as_mut()
            .ok_or_else(|| "No active recording".to_string())?;
        Ok(operation(manager))
    }

    pub fn set_paused(&self, paused: bool) -> Result<(), String> {
        let mut data = self.lock();
        let expected = if paused {
            SessionPhase::Recording
        } else {
            SessionPhase::Paused
        };
        if data.phase != expected {
            return Err(format!("Invalid pause transition from {:?}", data.phase));
        }

        let manager = data
            .manager
            .as_ref()
            .ok_or_else(|| "Active recording has no manager".to_string())?;
        if paused {
            manager
                .pause_recording()
                .map_err(|error| error.to_string())?;
        } else {
            manager
                .resume_recording()
                .map_err(|error| error.to_string())?;
        }

        data.phase = if paused {
            SessionPhase::Paused
        } else {
            SessionPhase::Recording
        };
        Ok(())
    }
}

pub static RECORDING_SESSION: LazyLock<RecordingSession> = LazyLock::new(RecordingSession::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_concurrent_start_and_stale_abort() {
        let session = RecordingSession::new();
        let generation = session.begin_start().unwrap();
        assert!(session.begin_start().is_err());
        session.abort_start(generation.wrapping_add(1));
        assert_eq!(session.phase(), SessionPhase::Starting);
        session.abort_start(generation);
        assert_eq!(session.phase(), SessionPhase::Idle);
    }

    #[test]
    fn rejects_stop_while_starting() {
        let session = RecordingSession::new();
        let generation = session.begin_start().unwrap();
        assert!(session.begin_stop().is_err());
        session.abort_start(generation);
    }

    #[tokio::test]
    async fn stop_resources_are_taken_once_and_stale_completion_is_ignored() {
        let session = RecordingSession::new();
        let generation = session.begin_start().unwrap();
        session
            .activate(generation, RecordingManager::new())
            .unwrap();
        session
            .install_transcription_task(generation, tokio::spawn(async {}))
            .unwrap();

        let resources = session.begin_stop().unwrap().unwrap();
        assert_eq!(resources.generation, generation);
        assert_eq!(session.phase(), SessionPhase::Stopping);
        assert!(session.begin_stop().is_err());

        session.finish_stop(generation.wrapping_add(1));
        assert_eq!(session.phase(), SessionPhase::Stopping);
        session.finish_stop(generation);
        assert_eq!(session.phase(), SessionPhase::Idle);
    }
}
