use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tokio::task::JoinHandle;

use super::{recording_state::RecordingState, RecordingManager};

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
        super::recording_commands::start_session(self, app, microphone, system_audio, meeting_name)
            .await
    }

    pub async fn stop<R: Runtime>(
        &self,
        app: AppHandle<R>,
    ) -> Result<Option<super::recording_commands::FinalizedRecording>, String> {
        super::recording_commands::finalize_session(self, app).await
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
