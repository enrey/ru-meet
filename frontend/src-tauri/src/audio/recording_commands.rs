// audio/recording_commands.rs
//
// Slim Tauri command layer for recording functionality.
// Delegates to transcription and recording modules for actual implementation.

use anyhow::Result;
use log::{debug, error, info, warn};
use serde::Serialize;
use std::sync::{atomic::Ordering, Arc};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::device_monitor::{DeviceEvent, DeviceMonitorType};
use super::{
    default_input_device,  // Get default microphone
    default_output_device, // Get default system audio
    parse_audio_device,
    recording_manager::RecordingStartError,
    RecordingManager,
};

// Import transcription modules
use super::transcription;

// Re-export TranscriptUpdate for backward compatibility
pub use super::transcription::TranscriptUpdate;

fn recording_live() -> bool {
    matches!(
        super::recording_session::RECORDING_SESSION.phase(),
        super::recording_session::SessionPhase::Recording
            | super::recording_session::SessionPhase::Paused
    )
}

/// Recording is live AND the authoritative manager still owns session `s`.
/// Used by the mic-disconnect fallback to refuse acting on a *later* recording
/// after a Stop/Start swapped the manager out from under an in-flight task.
///
fn session_live(s: &Arc<super::RecordingState>) -> bool {
    super::recording_session::RECORDING_SESSION.is_live_for_state(s)
}

const TRANSCRIPTION_RUNTIME_START_ERROR_CODE: &str = "TRANSCRIPTION_RUNTIME_INITIALIZATION_FAILED";
const TRANSCRIPTION_RUNTIME_USER_MESSAGE: &str = "Speech recognition could not initialize. Restart Ru-Meet. If the problem continues, repair or reinstall the app.";

// ============================================================================
// PUBLIC TYPES
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRecordingDevices {
    pub microphone: Option<String>,
    pub system: Option<String>,
}

pub use super::recording_sources::RecordingSourceMutes;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizedRecording {
    pub meeting_id: String,
    pub meeting_name: String,
    pub folder_path: Option<String>,
    pub transcript_count: usize,
    pub diarization_status: String,
}

pub(crate) async fn unload_transcription_engine<R: Runtime>(app: &AppHandle<R>) {
    transcription::unload_configured_engine(app).await;
}

/// Names of endpoints that the active recording actually opened. This differs
/// from a UI preference when "Default" resolved to a concrete device.
#[tauri::command]
pub fn get_active_recording_devices() -> Result<ActiveRecordingDevices, String> {
    super::recording_session::RECORDING_SESSION.with_manager(|manager| {
        let state = manager.get_state();
        ActiveRecordingDevices {
            microphone: state
                .get_microphone_device()
                .map(|device| device.name.clone()),
            system: state.get_system_device().map(|device| device.name.clone()),
        }
    })
}

#[tauri::command]
pub fn get_recording_source_mutes() -> Result<RecordingSourceMutes, String> {
    Ok(super::recording_sources::current())
}

#[tauri::command]
pub fn set_recording_source_muted<R: Runtime>(
    app: AppHandle<R>,
    source: String,
    muted: bool,
) -> Result<RecordingSourceMutes, String> {
    let device_type = match source.as_str() {
        "microphone" => super::recording_state::DeviceType::Microphone,
        "system" => super::recording_state::DeviceType::System,
        _ => return Err(format!("Unknown recording source: {source}")),
    };
    let mutes = super::recording_sources::set(&app, &source, muted)?;
    let _ = super::recording_session::RECORDING_SESSION.with_manager(|manager| {
        manager.get_state().set_source_muted(device_type, muted);
    });
    info!("Recording source '{}' muted: {}", source, muted);
    Ok(mutes)
}

pub(crate) fn map_recording_start_error<R: Runtime>(
    app: &AppHandle<R>,
    error: RecordingStartError,
) -> String {
    crate::tray::update_tray_menu(app);

    match error {
        RecordingStartError::TranscriptionRuntime(source) => {
            error!("Failed to initialize speech recognition: {source:#}");
            let error = RecordingStartError::TranscriptionRuntime(source);
            if let Err(emit_error) = app.emit(
                "transcription-error",
                serde_json::json!({
                    "error": error.to_string(),
                    "userMessage": TRANSCRIPTION_RUNTIME_USER_MESSAGE,
                    "actionable": false,
                    "phase": "startup"
                }),
            ) {
                error!("Failed to emit transcription runtime startup error: {emit_error}");
            }
            TRANSCRIPTION_RUNTIME_START_ERROR_CODE.to_string()
        }
        RecordingStartError::Other(error) => format!("Failed to start recording: {error}"),
    }
}

// ============================================================================
// DEVICE RESOLUTION
// ============================================================================

/// Resolve the microphone to record with: requested device (if it actually
/// enumerates) → system default → none (system-audio-only recording).
///
/// The device picker has no "no microphone" option: choosing "Default
/// Microphone" sends `None`, so `None` here means "use the system default",
/// NOT "record without a mic". A specifically-requested mic that isn't in
/// cpal's current enumeration (a stale saved device, or a Continuity
/// "iPhone Microphone" that isn't available right now) is downgraded to the
/// system default — the same `default_input_device()` helper the
/// mid-recording disconnect path uses — so start never hard-fails with
/// "Device not found".
///
/// Emits at most one event per call:
/// - `mic-device-switched` — a specific mic was requested but unavailable,
///   and we fell back to the default (reuses the existing frontend listener).
/// - `mic-unavailable` — no usable mic at all; recording proceeds with
///   system audio only. If system audio is also unavailable, start_streams'
///   own guard reports it.
/// Resolving `None` to the default is the user's actual choice, so it's silent.
///
/// ponytail: sync pre-flight substitution (matches Pro), not catch-and-retry —
/// stream.rs keeps its hard-fail as the last line of defense. cpal calls
/// block briefly either way.
pub(crate) fn resolve_mic_or_default<R: Runtime>(
    app: &AppHandle<R>,
    requested_name: Option<&str>,
) -> Option<Arc<super::AudioDevice>> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let requested_specific = requested_name.is_some();

    if let Some(name) = requested_name {
        match parse_audio_device(name) {
            Ok(device) => {
                let exists = cpal::default_host()
                    .input_devices()
                    .map(|mut it| it.any(|d| d.name().map(|n| n == device.name).unwrap_or(false)))
                    .unwrap_or(false);
                if exists {
                    info!("✅ Using requested microphone: '{}'", device.name);
                    return Some(Arc::new(device));
                }
                warn!(
                    "⚠️ Requested mic '{}' not enumerated — falling back to system default",
                    device.name
                );
            }
            Err(e) => {
                warn!(
                    "⚠️ Requested mic '{}' not available: {} — falling back to system default",
                    name, e
                );
            }
        }
    }

    match default_input_device() {
        Ok(device) => {
            info!("✅ Using default microphone: '{}'", device.name);
            if requested_specific {
                // Tell the user their selected mic wasn't available and which
                // mic is actually recording. Reuses the mic-device-switched
                // listener the disconnect path wires up.
                let _ = app.emit(
                    "mic-device-switched",
                    serde_json::json!({ "device_name": device.name }),
                );
            }
            Some(Arc::new(device))
        }
        Err(e) => {
            warn!(
                "❌ No microphone available: {} — recording system audio only",
                e
            );
            let _ = app.emit("mic-unavailable", serde_json::json!({}));
            None
        }
    }
}

/// System-audio analog of `resolve_mic_or_default`: `Some(name)` -> parse it,
/// falling back to the default output if unparseable; `None` ("Default System
/// Audio" in the UI) -> default output. Returns `None` only when no output
/// device exists — system audio is optional, mic-only recording proceeds.
///
/// ponytail: no cpal enumeration check (unlike the mic helper) — Linux system
/// devices are Pulse/ALSA monitor *inputs* tagged Output, so output_devices()
/// would false-negative them. stream.rs still hard-fails on a missing device.
pub(crate) fn resolve_system_or_default(
    requested_name: Option<&str>,
) -> Option<Arc<super::AudioDevice>> {
    if let Some(name) = requested_name {
        match parse_audio_device(name) {
            Ok(device) => {
                info!("✅ Using requested system audio: '{}'", device.name);
                return Some(Arc::new(device));
            }
            Err(e) => warn!(
                "⚠️ Requested system audio '{}' not available: {} — falling back to system default",
                name, e
            ),
        }
    }

    match default_output_device() {
        Ok(device) => {
            info!("✅ Using default system audio: '{}'", device.name);
            Some(Arc::new(device))
        }
        Err(e) => {
            warn!(
                "⚠️ No system audio available: {} — recording will continue with microphone only",
                e
            );
            None
        }
    }
}

/// Wake idle audio hardware before checking microphone callbacks, and finish
/// validation before creating any recording resources.
#[cfg(target_os = "macos")]
pub(crate) async fn prepare_audio_for_recording(
    system_device: Option<&super::AudioDevice>,
) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let wake_name = system_device.map(|s| s.name.clone()).or_else(|| {
        cpal::default_host()
            .default_output_device()
            .and_then(|d| d.name().ok())
    });
    if let Some(name) = wake_name {
        if let Err(e) = super::recording_manager::wake_audio_connection(&name).await {
            warn!("[AUDIO_WAKE] Wake failed: {} — proceeding anyway", e);
        }
    }

    if let Err(e) = super::devices::verify_microphone_access().await {
        error!("Microphone access verification failed: {}", e);
        return Err(format!("Microphone access required: {}", e));
    }
    Ok(())
}

// ============================================================================
// RECORDING COMMANDS
// ============================================================================

/// Change capture endpoints without restarting the meeting. New streams are
/// opened before old ones are retired, so a failed device open leaves the
/// existing recording untouched.
#[tauri::command]
pub async fn switch_recording_devices<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
) -> Result<(), String> {
    if !recording_live() {
        return Err("No active recording".into());
    }
    let mic = resolve_mic_or_default(&app, mic_device_name.as_deref());
    let system = resolve_system_or_default(system_device_name.as_deref());
    let session = super::recording_session::RECORDING_SESSION
        .with_manager(|manager| manager.get_state().clone())?;

    let new_mic = if let Some(device) = mic.clone() {
        Some((
            super::stream::AudioStream::create(
                device.clone(),
                session.clone(),
                super::recording_state::DeviceType::Microphone,
                None,
            )
            .await
            .map_err(|e| format!("Could not open microphone: {e}"))?,
            device,
        ))
    } else {
        None
    };
    let new_system = if let Some(device) = system.clone() {
        Some((
            super::stream::AudioStream::create(
                device.clone(),
                session.clone(),
                super::recording_state::DeviceType::System,
                None,
            )
            .await
            .map_err(|e| format!("Could not open system audio: {e}"))?,
            device,
        ))
    } else {
        None
    };
    let (old_mic, old_system) =
        super::recording_session::RECORDING_SESSION.with_manager_mut(|manager| {
            if !Arc::ptr_eq(manager.get_state(), &session) {
                return Err("Recording changed while switching devices".to_string());
            }
            Ok(manager.replace_streams(new_mic, new_system))
        })??;
    if let Some(stream) = old_mic {
        let _ = stream.stop();
    }
    if let Some(stream) = old_system {
        let _ = stream.stop();
    }
    app.emit(
        "recording-devices-switched",
        serde_json::json!({"microphone": mic_device_name, "system": system_device_name}),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Replace just the microphone capture endpoint. Kept separate from the
/// system-audio path so choosing speakers never triggers microphone fallback.
#[tauri::command]
pub async fn switch_recording_microphone<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
) -> Result<(), String> {
    if !recording_live() {
        return Err("No active recording".into());
    }
    let requested_name = mic_device_name
        .as_ref()
        .map(|name| format!("{} (input)", name));
    let device = resolve_mic_or_default(&app, requested_name.as_deref())
        .ok_or("No microphone is available")?;
    let session = super::recording_session::RECORDING_SESSION
        .with_manager(|manager| manager.get_state().clone())?;
    let stream = super::stream::AudioStream::create(
        device.clone(),
        session.clone(),
        super::recording_state::DeviceType::Microphone,
        None,
    )
    .await
    .map_err(|e| format!("Could not open microphone: {e}"))?;
    let old = super::recording_session::RECORDING_SESSION.with_manager_mut(|manager| {
        if !Arc::ptr_eq(manager.get_state(), &session) {
            return Err("Recording changed while switching microphone".to_string());
        }
        Ok(manager
            .replace_streams(Some((stream, device.clone())), None)
            .0)
    })??;
    if let Some(stream) = old {
        let _ = stream.stop();
    }
    app.emit(
        "recording-devices-switched",
        serde_json::json!({ "microphone": device.name }),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Replace just the Windows WASAPI loopback/system-audio endpoint. This must
/// not resolve or recreate the microphone: the two sources are independent.
#[tauri::command]
pub async fn switch_recording_system_audio<R: Runtime>(
    app: AppHandle<R>,
    system_device_name: Option<String>,
) -> Result<(), String> {
    if !recording_live() {
        return Err("No active recording".into());
    }
    let requested_name = system_device_name
        .as_ref()
        .map(|name| format!("{} (output)", name));
    let device = resolve_system_or_default(requested_name.as_deref())
        .ok_or("No system audio endpoint is available")?;
    let session = super::recording_session::RECORDING_SESSION
        .with_manager(|manager| manager.get_state().clone())?;
    let stream = super::stream::AudioStream::create(
        device.clone(),
        session.clone(),
        super::recording_state::DeviceType::System,
        None,
    )
    .await
    .map_err(|e| format!("Could not open system audio: {e}"))?;
    let old = super::recording_session::RECORDING_SESSION.with_manager_mut(|manager| {
        if !Arc::ptr_eq(manager.get_state(), &session) {
            return Err("Recording changed while switching system audio".to_string());
        }
        Ok(manager
            .replace_streams(None, Some((stream, device.clone())))
            .1)
    })??;
    if let Some(stream) = old {
        let _ = stream.stop();
    }
    app.emit(
        "recording-devices-switched",
        serde_json::json!({ "system": device.name }),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Start recording with specific devices and optional meeting name
pub async fn start_recording_with_devices_and_meeting<R: Runtime>(
    app: AppHandle<R>,
    mic_device_name: Option<String>,
    system_device_name: Option<String>,
    meeting_name: Option<String>,
) -> Result<(), String> {
    super::recording_session::RECORDING_SESSION
        .start(app, mic_device_name, system_device_name, meeting_name)
        .await
}

/// Stop only returns after audio, transcription, diarization and database
/// persistence have reached a terminal state for this exact session.
pub async fn stop_recording<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<FinalizedRecording>, String> {
    super::recording_session::RECORDING_SESSION.stop(app).await
}

/// Check if recording is active
pub async fn is_recording() -> bool {
    super::recording_session::RECORDING_SESSION.is_active()
}

/// Pause the current recording
#[tauri::command]
pub async fn pause_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    info!("Pausing recording");

    super::recording_session::RECORDING_SESSION.set_paused(true)?;

    // Emit pause event to frontend
    app.emit(
        "recording-paused",
        serde_json::json!({
            "message": "Recording paused"
        }),
    )
    .map_err(|e| e.to_string())?;

    // Update tray menu to reflect paused state
    crate::tray::update_tray_menu(&app);

    info!("Recording paused successfully");
    Ok(())
}

/// Resume the current recording
#[tauri::command]
pub async fn resume_recording<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    info!("Resuming recording");

    super::recording_session::RECORDING_SESSION.set_paused(false)?;

    // Emit resume event to frontend
    app.emit(
        "recording-resumed",
        serde_json::json!({
            "message": "Recording resumed"
        }),
    )
    .map_err(|e| e.to_string())?;

    // Update tray menu to reflect resumed state
    crate::tray::update_tray_menu(&app);

    info!("Recording resumed successfully");
    Ok(())
}

/// Check if recording is currently paused
#[tauri::command]
pub async fn is_recording_paused() -> bool {
    super::recording_session::RECORDING_SESSION.phase()
        == super::recording_session::SessionPhase::Paused
}

/// Get detailed recording state
#[tauri::command]
pub async fn get_recording_state() -> serde_json::Value {
    let is_recording = super::recording_session::RECORDING_SESSION.is_active();
    if let Ok(value) = super::recording_session::RECORDING_SESSION.with_manager(|manager| {
        serde_json::json!({
            "is_recording": is_recording,
            "is_paused": manager.is_paused(),
            "is_active": manager.is_active(),
            "recording_duration": manager.get_recording_duration(),
            "active_duration": manager.get_active_recording_duration(),
            "total_pause_duration": manager.get_total_pause_duration(),
            "current_pause_duration": manager.get_current_pause_duration()
        })
    }) {
        value
    } else {
        serde_json::json!({
            "is_recording": is_recording,
            "is_paused": false,
            "is_active": false,
            "recording_duration": null,
            "active_duration": null,
            "total_pause_duration": 0.0,
            "current_pause_duration": null
        })
    }
}

/// Get the meeting folder path for the current recording
/// Returns the path if a meeting name was set and folder structure initialized
#[tauri::command]
pub async fn get_meeting_folder_path() -> Result<Option<String>, String> {
    Ok(super::recording_session::RECORDING_SESSION
        .with_manager(|manager| {
            manager
                .get_meeting_folder()
                .map(|p| p.to_string_lossy().to_string())
        })
        .unwrap_or(None))
}

/// Open the active recording's folder before the meeting is persisted.
#[tauri::command]
pub async fn open_active_recording_folder() -> Result<(), String> {
    let folder_path = get_meeting_folder_path()
        .await?
        .ok_or_else(|| "Recording folder is not available".to_string())?;
    if !std::path::Path::new(&folder_path).is_dir() {
        return Err(format!("Recording folder not found: {folder_path}"));
    }

    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let opener = "xdg-open";

    std::process::Command::new(opener)
        .arg(&folder_path)
        .spawn()
        .map_err(|error| format!("Failed to open folder: {error}"))?;
    Ok(())
}

/// Resolve a saved meeting's audio file so the transcript can play a phrase
/// from it, and make sure the webview is allowed to read that file.
///
/// Startup already opens the current recordings folder, but a meeting recorded
/// before the user moved that folder lives somewhere else, so grant the file
/// itself here too. Returns `None` when the meeting has no audio on disk,
/// which is how the UI decides whether to offer playback at all.
#[tauri::command]
pub async fn get_meeting_audio_path<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, crate::state::AppState>,
    meeting_id: String,
) -> Result<Option<String>, String> {
    let folder: Option<String> =
        sqlx::query_scalar("SELECT folder_path FROM meetings WHERE id = ?")
            .bind(&meeting_id)
            .fetch_optional(state.db_manager.pool())
            .await
            .map_err(|error| format!("Failed to look up meeting folder: {error}"))?
            .flatten();
    let Some(folder) = folder else {
        return Ok(None);
    };

    // Same candidate list the diarization re-run uses, so both agree on which
    // file is "the recording".
    let folder = std::path::PathBuf::from(folder);
    let Some(audio_path) = [
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
    .find(|path| path.is_file()) else {
        return Ok(None);
    };

    if let Err(error) = app.asset_protocol_scope().allow_file(&audio_path) {
        return Err(format!("Could not grant access to the recording: {error}"));
    }
    Ok(Some(audio_path.to_string_lossy().to_string()))
}

/// Get accumulated transcript segments from current recording session
/// Used for syncing frontend state after page reload during active recording
#[tauri::command]
pub async fn get_transcript_history(
) -> Result<Vec<crate::audio::recording_saver::TranscriptSegment>, String> {
    Ok(super::recording_session::RECORDING_SESSION
        .with_manager(RecordingManager::get_transcript_segments)
        .unwrap_or_default())
}

/// Get meeting name from current recording session
/// Used for syncing frontend state after page reload during active recording
#[tauri::command]
pub async fn get_recording_meeting_name() -> Result<Option<String>, String> {
    Ok(super::recording_session::RECORDING_SESSION
        .with_manager(RecordingManager::get_meeting_name)
        .unwrap_or(None))
}

// ============================================================================
// DEVICE MONITORING COMMANDS (AirPods/Bluetooth disconnect/reconnect support)
// ============================================================================

/// Get information about the active audio output device
/// Used to warn users about Bluetooth playback issues
#[tauri::command]
pub async fn get_active_audio_output() -> Result<super::playback_monitor::AudioOutputInfo, String> {
    super::playback_monitor::get_active_audio_output()
        .await
        .map_err(|e| format!("Failed to get audio output info: {}", e))
}

// ============================================================================
// MIC HOT-SWAP (disconnect recovery)
// ============================================================================

// Guard against concurrent mic hot-swap tasks. Only used by the disconnect
// fallback path (trigger_mic_fallback_to_default) — the "chase the new
// default" auto-swap has been removed.
static MIC_SWAP_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// Bounded retry budget for the disconnect fallback (P1 #2). Counts COMPLETED
// failed attempts; MIC_SWAP_IN_PROGRESS still prevents overlapping swaps.
pub(crate) static MIC_FALLBACK_FAILED_ATTEMPTS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);
const MAX_MIC_FALLBACK_ATTEMPTS: u32 = 3;

/// Perform mic hot-swap using phased locking — never holds session state during I/O
/// except the brief mic-stream stop in Phase 1.
/// If CPAL hangs during stream creation, only this task blocks; stop flow stays unblocked.
async fn perform_mic_hot_swap_task<R: Runtime>(
    new_device_name: String,
    session: &Arc<super::RecordingState>,
    app: AppHandle<R>,
) -> Result<(), String> {
    info!("[HOT_SWAP] Starting mic hot-swap to '{}'", new_device_name);

    match do_mic_swap(&new_device_name, session).await {
        Ok(()) => {
            info!("[HOT_SWAP] Mic switched to '{}'", new_device_name);
            let _ = app.emit(
                "mic-device-switched",
                serde_json::json!({
                    "device_name": new_device_name
                }),
            );
            Ok(())
        }
        Err(e) => {
            if !session_live(session) {
                return Err(e);
            }
            warn!("[HOT_SWAP] First attempt failed: {} — retrying in 500ms", e);
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            match do_mic_swap(&new_device_name, session).await {
                Ok(()) => {
                    info!("[HOT_SWAP] Mic switched to '{}' on retry", new_device_name);
                    let _ = app.emit(
                        "mic-device-switched",
                        serde_json::json!({
                            "device_name": new_device_name
                        }),
                    );
                    Ok(())
                }
                Err(e) => {
                    error!("[HOT_SWAP] Mic swap failed after retry: {}", e);
                    if session_live(session) {
                        let _ = app.emit(
                            "mic-swap-failed",
                            serde_json::json!({
                                "error": e,
                                "device_name": new_device_name
                            }),
                        );
                    }
                    Err(e)
                }
            }
        }
    }
}

/// Phased mic swap — lock is never held during async I/O.
async fn do_mic_swap(
    device_name: &str,
    session: &Arc<super::RecordingState>,
) -> Result<(), String> {
    // Phase 1: Lock briefly — verify identity, take old stream OUT (no teardown under lock)
    let old_mic = super::recording_session::RECORDING_SESSION.with_manager_mut(|manager| {
        if !manager.is_recording() {
            return Err("Recording stopped — aborting mic hot-swap".to_string());
        }
        if !Arc::ptr_eq(manager.get_state(), session) {
            return Err("Session changed before hot-swap — aborting".to_string());
        }
        Ok(manager.take_mic_stream_for_swap())
    })??;

    // Tear down the dead mic OUTSIDE the lock — cpal stop()/drop on a
    // disconnected BT device can stall on the CoreAudio HAL lock; doing it
    // while holding session state would freeze stop_recording.
    // Non-fatal: the replacement stream is created next regardless, so a
    // teardown error/stall on the already-dead device must not abort the swap.
    if let Some(s) = old_mic {
        if let Err(e) = s.stop() {
            warn!(
                "[HOT_SWAP] Failed to stop old mic stream (proceeding): {}",
                e
            );
        }
    }

    // Phase 2: Async I/O WITHOUT lock — may be slow, that's OK
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Build the AudioDevice directly from the name — the caller
    // (trigger_mic_fallback_to_default) already resolved it via
    // default_input_device(). Skipping list_audio_devices() here avoids a
    // full cpal enumeration on the exact BT-transition hot path where it's
    // known to hang 100+ s (see H2 in PR-175 review). The real device
    // validation happens inside AudioStream::create → get_device_and_config
    // which does a targeted host.input_devices() lookup by name.
    let device_arc = std::sync::Arc::new(super::AudioDevice::new(
        device_name.to_string(),
        super::DeviceType::Input,
    ));

    info!(
        "[HOT_SWAP] Creating new mic stream for '{}' (lock released)",
        device_name
    );
    let new_stream = super::stream::AudioStream::create(
        device_arc.clone(),
        session.clone(),
        super::recording_state::DeviceType::Microphone,
        None,
    )
    .await
    .map_err(|e| format!("Failed to create mic stream: {}", e))?;

    // Resolve the current default output OUTSIDE the lock — a CoreAudio stall here
    // must not block stop_recording (which needs the session state).
    let system_name = default_output_device().ok().map(|d| d.name);

    // Phase 3: Lock briefly — install ONLY if still the same session
    super::recording_session::RECORDING_SESSION.with_manager_mut(|manager| {
        if !Arc::ptr_eq(manager.get_state(), session) {
            return Err(
                "Session changed during hot-swap — discarding stale mic stream".to_string(),
            );
        }
        manager.set_mic_stream_after_swap(new_stream, device_arc, system_name);
        info!("[HOT_SWAP] Mic hot-swap to '{}' completed", device_name);
        Ok(())
    })??;

    Ok(())
}

/// Background processor for device monitor events during a recording session.
///
/// The ONLY mid-recording mic switch that is allowed is the fallback from a
/// dead device to the system default, triggered by the device monitor's
/// DeviceDisconnected event. Any other device event is explicitly ignored —
/// recording stays on whatever device was picked at start time until the
/// meeting ends.
///
/// Rationale: auto-swapping to a freshly-connected BT device during recording
/// triggers a reliable hang inside cpal's stream creation on macOS. Locking
/// the device at start eliminates that hang and also makes the recording
/// session predictable.
///
/// The task stops automatically when the receiver is dropped (recording
/// ends / monitor stops).
pub(crate) fn spawn_device_event_processor<R: Runtime>(
    app: AppHandle<R>,
    mut receiver: tokio::sync::mpsc::UnboundedReceiver<DeviceEvent>,
    session: Arc<super::RecordingState>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("[DEVICE_EVENTS] Background event processor started");

        while let Some(event) = receiver.recv().await {
            // Skip if recording has stopped
            if !recording_live() {
                info!(
                    "[DEVICE_EVENTS] Recording stopped — ignoring event: {:?}",
                    event
                );
                continue;
            }

            match event {
                DeviceEvent::DeviceDisconnected {
                    ref device_name,
                    ref device_type,
                } => {
                    info!(
                        "[DEVICE_EVENTS] Device disconnected: '{}' ({:?})",
                        device_name, device_type
                    );
                    // The only automatic mid-recording mic change allowed:
                    // when the active microphone dies, fall back to the
                    // system default input. Triggered after the device
                    // monitor's polling threshold fires.
                    if matches!(device_type, DeviceMonitorType::Microphone) {
                        let name = device_name.clone();
                        let app_clone = app.clone();
                        let session = session.clone();
                        tokio::spawn(async move {
                            trigger_mic_fallback_to_default(app_clone, name, session).await;
                        });
                    }
                }
                DeviceEvent::DeviceReconnected {
                    ref device_name,
                    ref device_type,
                } => {
                    // Per product decision: once we have fallen back to the
                    // built-in mic we stay there for the rest of the meeting.
                    // This is intentional — just log and do nothing.
                    info!("[DEVICE_EVENTS] Device reconnected: '{}' ({:?}) — staying on current mic (fallback is sticky)", device_name, device_type);
                }
                DeviceEvent::DeviceListChanged => {
                    debug!("[DEVICE_EVENTS] Device list changed");
                }
            }
        }
        info!("[DEVICE_EVENTS] Background event processor stopped (channel closed)");
    })
}

/// Disconnect fallback: swap the active mic to the system default input
/// device. Triggered from the background device event processor after the
/// device monitor's polling threshold (3 × 2s) fires `DeviceDisconnected`
/// for the active microphone.
///
/// `disconnected_name` is the device that just died. We keep it to detect
/// the edge case where macOS hasn't yet updated the system default input
/// away from the dead device — we wait and retry in that case rather than
/// swapping back to the same broken device.
///
/// This function takes the MIC_SWAP_IN_PROGRESS guard itself; the caller
/// must NOT already hold it. If a swap is somehow already running this
/// returns immediately.
async fn trigger_mic_fallback_to_default<R: Runtime>(
    app: AppHandle<R>,
    disconnected_name: String,
    session: Arc<super::RecordingState>,
) {
    if !session_live(&session) {
        info!(
            "[MIC_FALLBACK] Not recording — skipping fallback for '{}'",
            disconnected_name
        );
        return;
    }

    if MIC_FALLBACK_FAILED_ATTEMPTS.load(Ordering::SeqCst) >= MAX_MIC_FALLBACK_ATTEMPTS {
        warn!(
            "[MIC_FALLBACK] {} failed attempts reached — giving up on '{}' (terminal event already announced)",
            MAX_MIC_FALLBACK_ATTEMPTS, disconnected_name
        );
        return;
    }

    if MIC_SWAP_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        info!(
            "[MIC_FALLBACK] Swap already in progress — skipping fallback for '{}'",
            disconnected_name
        );
        return;
    }

    // Guard that clears MIC_SWAP_IN_PROGRESS on any return path below so a
    // panic or early return can't leave the flag stuck.
    struct SwapGuard;
    impl Drop for SwapGuard {
        fn drop(&mut self) {
            MIC_SWAP_IN_PROGRESS.store(false, Ordering::SeqCst);
        }
    }
    let _guard = SwapGuard;

    info!(
        "[MIC_FALLBACK] Starting fallback from disconnected device '{}'",
        disconnected_name
    );

    // Let macOS finish swapping the system default input away from the dead
    // device. 150ms is enough in practice for the built-in mic to become the
    // default when an explicitly-selected BT device disconnects.
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // A Stop-A/Start-B during the sleep swapped our session out. Bail silently —
    // emitting or spending the recovery budget here would fire against B with
    // A's device. Covers the default_input_device() error branch below.
    if !session_live(&session) {
        info!(
            "[MIC_FALLBACK] Session no longer live after wait — aborting fallback for '{}'",
            disconnected_name
        );
        return;
    }

    // Query the current system default input. If it still reports the
    // disconnected device, back off once more and re-query — this handles
    // the edge case where the OS hasn't propagated the change yet.
    let fallback_name = match default_input_device() {
        Ok(dev) => dev.name,
        Err(e) => {
            error!("[MIC_FALLBACK] Failed to query default input device: {}", e);
            let _ = app.emit(
                "mic-swap-failed",
                serde_json::json!({
                    "error": format!("Failed to query default input: {}", e),
                    "device_name": disconnected_name,
                }),
            );
            let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
            if n == MAX_MIC_FALLBACK_ATTEMPTS {
                let _ = app.emit(
                    "mic-recovery-exhausted",
                    serde_json::json!({ "device_name": disconnected_name }),
                );
            }
            return;
        }
    };

    let fallback_name = if fallback_name == disconnected_name {
        warn!(
            "[MIC_FALLBACK] Default input still reports disconnected device '{}' — retrying after 300ms",
            disconnected_name
        );
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        if !session_live(&session) {
            info!("[MIC_FALLBACK] Session no longer live after retry wait — aborting fallback for '{}'", disconnected_name);
            return;
        }
        match default_input_device() {
            Ok(dev) if dev.name != disconnected_name => dev.name,
            Ok(dev) => {
                error!(
                    "[MIC_FALLBACK] Default input still '{}' after retry — aborting fallback",
                    dev.name
                );
                let _ = app.emit(
                    "mic-swap-failed",
                    serde_json::json!({
                        "error": "System default input still reports disconnected device after retry",
                        "device_name": disconnected_name,
                    }),
                );
                let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
                if n == MAX_MIC_FALLBACK_ATTEMPTS {
                    let _ = app.emit(
                        "mic-recovery-exhausted",
                        serde_json::json!({ "device_name": disconnected_name }),
                    );
                }
                return;
            }
            Err(e) => {
                error!(
                    "[MIC_FALLBACK] Failed to re-query default input device: {}",
                    e
                );
                let _ = app.emit(
                    "mic-swap-failed",
                    serde_json::json!({
                        "error": format!("Failed to re-query default input: {}", e),
                        "device_name": disconnected_name,
                    }),
                );
                let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
                if n == MAX_MIC_FALLBACK_ATTEMPTS {
                    let _ = app.emit(
                        "mic-recovery-exhausted",
                        serde_json::json!({ "device_name": disconnected_name }),
                    );
                }
                return;
            }
        }
    } else {
        fallback_name
    };

    info!(
        "[MIC_FALLBACK] Falling back '{}' → '{}'",
        disconnected_name, fallback_name
    );

    // macOS Core Audio pre-wake for the hot-swap path — before we call the
    // rebuild path (which internally calls `AudioDeviceStart` on the new
    // mic), play 150ms of digital silence through the current system
    // output device to force the Core Audio hardware unit out of its idle
    // power state. Without this, `AudioDeviceStart` can return `noErr` but
    // the IO proc will not fire for 10-30 seconds until some other audio
    // nudges the hardware awake — the "backend idle until you play YouTube"
    // symptom from earlier testing.
    //
    // `wake_audio_connection_for_swap` has a built-in fallback: if the
    // current system device name doesn't enumerate (e.g. the BT output just
    // disappeared), it plays through `default_output_device()` instead,
    // which on macOS will now be the built-in speakers — exactly the
    // hardware unit we want to wake for the fallback mic.
    //
    // Non-fatal: on error we log and proceed to the swap anyway. A failed
    // wake is strictly better than no wake.
    #[cfg(target_os = "macos")]
    {
        // Read from the captured session directly (no manager lock) — this
        // stays correct even if the global manager has since been swapped by
        // a Stop/Start of a different session.
        let sys_device_name = session.get_system_device().map(|d| d.name.clone());
        if let Some(name) = sys_device_name {
            match super::recording_manager::wake_audio_connection_for_swap(&name).await {
                Ok(()) => info!("[MIC_FALLBACK] Pre-swap audio wake completed"),
                Err(e) => warn!(
                    "[MIC_FALLBACK] Pre-swap audio wake failed: {} — proceeding anyway",
                    e
                ),
            }
        } else {
            log::debug!("[MIC_FALLBACK] No system device recorded — skipping pre-swap wake");
        }
    }

    // Stop may have started during the sleeps above — bail before touching
    // the (possibly already taken) manager.
    if !session_live(&session) {
        info!(
            "[MIC_FALLBACK] Recording stopping — aborting fallback for '{}'",
            disconnected_name
        );
        return;
    }

    // perform_mic_hot_swap_task performs its own retry-once logic on failure
    // and emits the mic-device-switched / mic-swap-failed events, so we can
    // just delegate here. It does NOT touch MIC_SWAP_IN_PROGRESS internally.
    match perform_mic_hot_swap_task(fallback_name.clone(), &session, app.clone()).await {
        Ok(()) => {
            info!(
                "[MIC_FALLBACK] Fallback complete: now recording via '{}'",
                fallback_name
            );
            MIC_FALLBACK_FAILED_ATTEMPTS.store(0, Ordering::SeqCst);
        }
        Err(e) => {
            error!("[MIC_FALLBACK] Fallback swap failed: {}", e);
            if !session_live(&session) {
                return;
            }
            let n = MIC_FALLBACK_FAILED_ATTEMPTS.fetch_add(1, Ordering::SeqCst) + 1;
            if n == MAX_MIC_FALLBACK_ATTEMPTS {
                let _ = app.emit(
                    "mic-recovery-exhausted",
                    serde_json::json!({ "device_name": disconnected_name }),
                );
            }
        }
    }
}
