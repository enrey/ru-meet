use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_store::StoreExt;

mod monitor;
pub(crate) mod microphone_activity;
pub(crate) mod preview_vad;
#[cfg(target_os = "windows")]
mod preview_keeper;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSettings {
    #[serde(default = "default_enabled")]
    pub auto_record_meetings: bool,
    #[serde(default)]
    pub excluded_apps: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

impl Default for AutomationSettings {
    fn default() -> Self {
        Self {
            auto_record_meetings: true,
            excluded_apps: Vec::new(),
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
    /// The detector waits for silence after a manual stop before listening again.
    paused_after_manual_stop: bool,
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

/// Automatic recording is enabled, supported here, has a source to listen to
/// and is not paused after a manual stop — i.e. it would start on speech.
pub(crate) fn is_listening<R: Runtime>(app: &AppHandle<R>) -> bool {
    let enabled = app
        .try_state::<AutomationState>()
        .is_some_and(|state| state.0.read().is_ok_and(|settings| settings.auto_record_meetings));
    let mutes = crate::audio::recording_sources::current();
    cfg!(target_os = "windows")
        && enabled
        && !(mutes.microphone && mutes.system)
        && !monitor::paused_after_manual_stop()
}

#[tauri::command]
pub fn get_automation_preferences<R: Runtime>(
    app: AppHandle<R>,
) -> Result<AutomationPreferences, String> {
    let state = app
        .try_state::<AutomationState>()
        .ok_or("Automation settings are unavailable")?;
    let settings = state.0.read().map_err(|e| e.to_string())?;
    Ok(AutomationPreferences {
        auto_record_meetings: settings.auto_record_meetings,
        launch_at_login: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
        auto_record_supported: cfg!(target_os = "windows"),
        excluded_apps: settings.excluded_apps.clone(),
        paused_after_manual_stop: monitor::paused_after_manual_stop(),
    })
}

#[tauri::command]
pub fn set_auto_record_meetings<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let state = app
        .try_state::<AutomationState>()
        .ok_or("Automation settings are unavailable")?;
    let mut settings = state.0.write().map_err(|e| e.to_string())?;
    let next = AutomationSettings {
        auto_record_meetings: enabled,
        ..settings.clone()
    };
    let store = app
        .store(crate::portable::store_path("automation.json"))
        .map_err(|e| e.to_string())?;
    store.set(
        "settings",
        serde_json::to_value(&next).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    *settings = next;
    drop(settings);
    let _ = app.emit("auto-record-meetings-changed", enabled);
    crate::tray::refresh_tray_indicator(&app);
    Ok(())
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
    let names = normalize_exclusions(apps)?;
    let state = app
        .try_state::<AutomationState>()
        .ok_or("Automation settings are unavailable")?;
    let mut settings = state.0.write().map_err(|e| e.to_string())?;
    let next = AutomationSettings {
        excluded_apps: names.clone(),
        ..settings.clone()
    };
    let store = app
        .store(crate::portable::store_path("automation.json"))
        .map_err(|e| e.to_string())?;
    store.set(
        "settings",
        serde_json::to_value(&next).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())?;
    *settings = next;
    Ok(names)
}

fn normalize_exclusions(apps: Vec<String>) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for name in apps {
        let name = name.trim().to_lowercase();
        if name.is_empty() || name.contains(['/', '\\', ':']) || !name.ends_with(".exe") {
            return Err("Enter an application filename, such as Spotify.exe".into());
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exclusions_are_case_insensitive_and_reject_paths() {
        assert_eq!(
            normalize_exclusions(vec![" Spotify.exe ".into(), "spotify.EXE".into()]).unwrap(),
            vec!["spotify.exe"]
        );
        assert!(normalize_exclusions(vec![r"C:\Apps\Spotify.exe".into()]).is_err());
        assert!(normalize_exclusions(vec!["spotify".into()]).is_err());
        assert!(normalize_exclusions(vec!["".into()]).is_err());
        assert!(normalize_exclusions(Vec::new()).unwrap().is_empty());
    }
    #[test]
    fn fresh_and_older_preferences_enable_recording_by_default() {
        assert!(AutomationSettings::default().auto_record_meetings);
        let settings: AutomationSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.auto_record_meetings);
        assert!(settings.excluded_apps.is_empty());
        let settings: AutomationSettings =
            serde_json::from_str(r#"{"autoRecordMeetings":false,"excludedApps":["spotify.exe"]}"#)
                .unwrap();
        assert!(!settings.auto_record_meetings);
        assert_eq!(settings.excluded_apps, vec!["spotify.exe"]);
    }
}
