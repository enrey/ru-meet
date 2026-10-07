use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecordingSourceMutes {
    pub microphone: bool,
    pub system: bool,
}

static MUTES: Mutex<RecordingSourceMutes> = Mutex::new(RecordingSourceMutes {
    microphone: false,
    system: false,
});

pub fn current() -> RecordingSourceMutes {
    *MUTES.lock().unwrap_or_else(|error| error.into_inner())
}

pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let store = app.store(crate::portable::store_path("recording_preferences.json"))
        .map_err(|error| error.to_string())?;
    if let Some(value) = store.get("source_mutes") {
        let mutes = serde_json::from_value(value).map_err(|error| error.to_string())?;
        *MUTES.lock().unwrap_or_else(|error| error.into_inner()) = mutes;
    }
    Ok(())
}

pub fn set<R: Runtime>(app: &AppHandle<R>, source: &str, muted: bool) -> Result<RecordingSourceMutes, String> {
    let mut saved = MUTES.lock().unwrap_or_else(|error| error.into_inner());
    let mut next = *saved;
    match source {
        "microphone" => next.microphone = muted,
        "system" => next.system = muted,
        _ => return Err(format!("Unknown recording source: {source}")),
    }
    let store = app.store(crate::portable::store_path("recording_preferences.json"))
        .map_err(|error| error.to_string())?;
    store.set("source_mutes", serde_json::to_value(next).map_err(|error| error.to_string())?);
    if let Err(error) = store.save() {
        store.set("source_mutes", serde_json::to_value(*saved).map_err(|error| error.to_string())?);
        return Err(error.to_string());
    }
    *saved = next;
    Ok(next)
}
