//! Release notes shown in the About dialog and the update dialog.
//!
//! The app avoids extra network traffic: release notes are kept in a local
//! cache file that is filled from data the app already downloads. An update
//! check reads `latest.json`, whose `releases` history
//! (`.github/workflows/release-notes.yml`) the frontend saves here with
//! `save_release_history`. The GitHub Releases API is contacted only when the
//! user explicitly asks (`fetch_release_history`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Runtime};

const RELEASES_URL: &str = "https://api.github.com/repos/enrey/ru-meet/releases?per_page=15";
const CACHE_FILE: &str = "release_notes.json";

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    draft: bool,
    prerelease: bool,
}

/// One entry of the `releases` array in `latest.json`.
#[derive(Serialize, Deserialize)]
pub struct ReleaseNotesEntry {
    version: String,
    pub_date: Option<String>,
    notes: String,
}

fn cache_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|dir| dir.join(CACHE_FILE))
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))
}

fn write_cache<R: Runtime>(app: &AppHandle<R>, releases: &[ReleaseNotesEntry]) -> Result<(), String> {
    let path = cache_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec_pretty(releases).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

/// Locally cached release notes, newest first; empty when nothing was cached yet.
#[tauri::command]
pub async fn get_cached_release_history<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<ReleaseNotesEntry>, String> {
    let path = cache_path(&app)?;
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).or_else(|e| {
            log::warn!("Ignoring unreadable release notes cache {}: {e}", path.display());
            Ok(Vec::new())
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("Failed to read {}: {e}", path.display())),
    }
}

/// Stores the `releases` history from `latest.json` fetched by an update check.
#[tauri::command]
pub async fn save_release_history<R: Runtime>(
    app: AppHandle<R>,
    releases: Vec<ReleaseNotesEntry>,
) -> Result<(), String> {
    write_cache(&app, &releases)
}

/// Downloads published, non-prerelease releases (newest first) from GitHub and
/// caches them. Called only on explicit user action.
#[tauri::command]
pub async fn fetch_release_history<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<ReleaseNotesEntry>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("ru-meet-release-notes")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let releases: Vec<GithubRelease> = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("Failed to fetch releases: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse releases: {e}"))?;

    let history: Vec<ReleaseNotesEntry> = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .map(|release| ReleaseNotesEntry {
            version: release.tag_name.trim_start_matches('v').to_string(),
            pub_date: release.published_at,
            notes: release.body.unwrap_or_default(),
        })
        .collect();

    if let Err(e) = write_cache(&app, &history) {
        log::warn!("Failed to cache release notes: {e}");
    }
    Ok(history)
}
