//! Automatic meeting recording (ADR 0004): a call app capturing the microphone
//! starts a recording; the in-person mode adds the older VAD trigger.
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_store::StoreExt;

pub(crate) mod call_apps;
#[cfg(target_os = "macos")]
mod macos;
pub(crate) mod microphone_activity;
mod monitor;
mod preview_keeper;
pub(crate) mod preview_vad;
pub(crate) mod recording_speech;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSettings {
    #[serde(default = "default_enabled")]
    pub auto_record_meetings: bool,
    /// Apps that never count as a call, nor as playback in the in-person mode.
    #[serde(default)]
    pub excluded_apps: Vec<String>,
    /// Call apps in addition to `call_apps::DEFAULT_CALL_APPS`.
    #[serde(default)]
    pub call_apps: Vec<String>,
    /// Any app capturing the microphone counts as a call.
    #[serde(default)]
    pub record_any_app: bool,
    /// Also start on confirmed speech without a call; keeps the microphone open.
    #[serde(default)]
    pub record_in_person: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self {
            auto_record_meetings: true,
            excluded_apps: Vec::new(),
            call_apps: Vec::new(),
            record_any_app: false,
            record_in_person: false,
        }
    }
}

pub struct AutomationState(pub std::sync::RwLock<AutomationSettings>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationPreferences {
    auto_record_meetings: bool,
    launch_at_login: bool,
    auto_record_supported: bool,
    excluded_apps: Vec<String>,
    call_apps: Vec<String>,
    record_any_app: bool,
    record_in_person: bool,
    /// The detector waits for the call app to release the microphone (and,
    /// in the in-person mode, for silence) after a manual stop, a manual
    /// recording or a backstop.
    paused_until_call_ends: bool,
    /// Listing the processes that capture the microphone fails right now.
    detection_failed: bool,
    /// Processes capturing the microphone right now, for finding app names.
    mic_holders: Vec<call_apps::MicHolder>,
}

/// Processes capturing the microphone; `None` where detection does not exist.
pub(crate) fn mic_clients() -> Option<Result<Vec<call_apps::AudioClient>, String>> {
    #[cfg(target_os = "windows")]
    return Some(windows::mic_clients());
    #[cfg(target_os = "macos")]
    return Some(macos::mic_clients());
    #[allow(unreachable_code)]
    None
}

/// Whether this system can tell which processes capture the microphone.
/// Without that our own capture cannot be told from a call, so automatic
/// recording stays off (macOS before 14.4, Linux).
pub(crate) fn is_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| match mic_clients() {
        Some(Ok(_)) => true,
        Some(Err(error)) => {
            log::warn!("Automatic recording is unavailable: {error}");
            false
        }
        None => false,
    })
}

pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let store = app
        .store(crate::portable::store_path("automation.json"))
        .map_err(|e| e.to_string())?;
    let settings = match store.get("settings") {
        Some(value) => serde_json::from_value(value).map_err(|e| e.to_string())?,
        None => AutomationSettings::default(),
    };
    app.manage(AutomationState(std::sync::RwLock::new(settings)));
    monitor::start(app.clone());
    Ok(())
}

/// Automatic recording is enabled, supported here, has a source to record
/// and is not paused until the call ends — i.e. it would start on a call.
pub(crate) fn is_listening<R: Runtime>(app: &AppHandle<R>) -> bool {
    let enabled = app
        .try_state::<AutomationState>()
        .is_some_and(|state| state.0.read().is_ok_and(|settings| settings.auto_record_meetings));
    let mutes = crate::audio::recording_sources::current();
    enabled
        && !(mutes.microphone && mutes.system)
        && !monitor::paused_until_call_ends()
        && !monitor::detection_failed()
        && is_supported()
}

#[tauri::command]
pub fn get_automation_preferences<R: Runtime>(
    app: AppHandle<R>,
) -> Result<AutomationPreferences, String> {
    let state = app
        .try_state::<AutomationState>()
        .ok_or("Automation settings are unavailable")?;
    let settings = state.0.read().map_err(|e| e.to_string())?.clone();
    Ok(AutomationPreferences {
        auto_record_meetings: settings.auto_record_meetings,
        launch_at_login: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
        auto_record_supported: is_supported(),
        excluded_apps: settings.excluded_apps,
        call_apps: settings.call_apps,
        record_any_app: settings.record_any_app,
        record_in_person: settings.record_in_person,
        paused_until_call_ends: monitor::paused_until_call_ends(),
        detection_failed: monitor::detection_failed(),
        mic_holders: monitor::mic_holders(),
    })
}

/// Apply `change`, persist the result and only then publish it in memory.
fn update_settings<R: Runtime>(
    app: &AppHandle<R>,
    change: impl FnOnce(&mut AutomationSettings),
) -> Result<AutomationSettings, String> {
    let state = app
        .try_state::<AutomationState>()
        .ok_or("Automation settings are unavailable")?;
    let mut settings = state.0.write().map_err(|e| e.to_string())?;
    let mut next = settings.clone();
    change(&mut next);
    let store = app
        .store(crate::portable::store_path("automation.json"))
        .map_err(|e| e.to_string())?;
    store.set(
        "settings",
        serde_json::to_value(&next).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    *settings = next.clone();
    Ok(next)
}

#[tauri::command]
pub fn set_auto_record_meetings<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    update_settings(&app, |settings| settings.auto_record_meetings = enabled)?;
    let _ = app.emit("auto-record-meetings-changed", enabled);
    crate::tray::refresh_tray_indicator(&app);
    Ok(())
}

/// `option` is `anyApp` or `inPerson`.
#[tauri::command]
pub fn set_auto_record_option<R: Runtime>(
    app: AppHandle<R>,
    option: String,
    enabled: bool,
) -> Result<(), String> {
    let field: fn(&mut AutomationSettings) -> &mut bool = match option.as_str() {
        "anyApp" => |settings| &mut settings.record_any_app,
        "inPerson" => |settings| &mut settings.record_in_person,
        _ => return Err(format!("Unknown automatic recording option: {option}")),
    };
    let settings = update_settings(&app, |settings| *field(settings) = enabled)?;
    let _ = app.emit("auto-record-meetings-changed", settings.auto_record_meetings);
    Ok(())
}

/// The in-app meeting player is playing a recording. That is speech too, both
/// in the playback loopback and through the microphone, so it must not start
/// an automatic recording in the in-person mode.
#[tauri::command]
pub fn set_meeting_playback_active(active: bool) {
    monitor::set_meeting_playback_active(active);
}

#[tauri::command]
pub fn set_launch_at_login<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<bool, String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())?;
    launcher.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_auto_record_excluded_apps<R: Runtime>(
    app: AppHandle<R>,
    apps: Vec<String>,
) -> Result<Vec<String>, String> {
    let names = call_apps::normalize_app_list(apps)?;
    Ok(update_settings(&app, |settings| settings.excluded_apps = names)?.excluded_apps)
}

#[tauri::command]
pub fn set_auto_record_call_apps<R: Runtime>(
    app: AppHandle<R>,
    apps: Vec<String>,
) -> Result<Vec<String>, String> {
    let names = call_apps::normalize_app_list(apps)?;
    Ok(update_settings(&app, |settings| settings.call_apps = names)?.call_apps)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_and_older_preferences_enable_recording_by_default() {
        assert!(AutomationSettings::default().auto_record_meetings);
        let settings: AutomationSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.auto_record_meetings);
        assert!(settings.excluded_apps.is_empty());
        assert!(!settings.record_in_person);
        assert!(!settings.record_any_app);
        let settings: AutomationSettings =
            serde_json::from_str(r#"{"autoRecordMeetings":false,"excludedApps":["spotify.exe"]}"#)
                .unwrap();
        assert!(!settings.auto_record_meetings);
        assert_eq!(settings.excluded_apps, vec!["spotify.exe"]);
        assert!(settings.call_apps.is_empty());
    }
}
