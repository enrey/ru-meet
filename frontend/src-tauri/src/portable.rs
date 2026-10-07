//! Optional Windows portable storage, enabled by portable.json beside the executable.

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::path::{Component, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableConfig {
    data_dir: PathBuf,
}

static DATA_DIR: Lazy<Option<PathBuf>> = Lazy::new(|| {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("MEETILY_DEV_DATA_DIR") {
        let path = PathBuf::from(path);
        assert!(path.is_absolute(), "MEETILY_DEV_DATA_DIR must be absolute");
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|error| panic!("Cannot create {}: {error}", path.display()));
        return Some(path);
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }

    #[cfg(target_os = "windows")]
    {
        let executable = std::env::current_exe().expect("Cannot locate Ru-Meet executable");
        let executable_dir = executable
            .parent()
            .expect("Executable has no parent directory");
        let config_path = executable_dir.join("portable.json");
        if !config_path.exists() {
            return None;
        }

        let contents = std::fs::read_to_string(&config_path)
            .unwrap_or_else(|error| panic!("Cannot read {}: {error}", config_path.display()));
        let config: PortableConfig = serde_json::from_str(&contents)
            .unwrap_or_else(|error| panic!("Invalid {}: {error}", config_path.display()));
        if config.data_dir.as_os_str().is_empty()
            || !config
                .data_dir
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            panic!("portable.json dataDir must be a relative path without parent components");
        }
        let data_dir = executable_dir.join(config.data_dir);
        std::fs::create_dir_all(&data_dir)
            .unwrap_or_else(|error| panic!("Cannot create {}: {error}", data_dir.display()));
        Some(data_dir)
    }
});

/// Per-user folder name outside a portable install.
const PRODUCT_DIR: &str = "Ru-Meet";
/// Name used before the Ru-Meet rename.
const LEGACY_PRODUCT_DIR: &str = "Meetily";
const RECORDINGS_DIR: &str = "ru-meet-recordings";
const LEGACY_RECORDINGS_DIR: &str = "meetily-recordings";

/// `base/Ru-Meet`, or `base/Meetily` when an install from before the rename
/// already keeps data there, so models (hundreds of MB), templates and other
/// files are neither stranded nor downloaded again.
pub fn product_dir(base: PathBuf) -> PathBuf {
    prefer_existing(base, LEGACY_PRODUCT_DIR, PRODUCT_DIR)
}

/// Default recordings folder inside `base`, with the same legacy rule.
pub fn recordings_dir(base: PathBuf) -> PathBuf {
    prefer_existing(base, LEGACY_RECORDINGS_DIR, RECORDINGS_DIR)
}

fn prefer_existing(base: PathBuf, legacy: &str, current: &str) -> PathBuf {
    let legacy_path = base.join(legacy);
    if legacy_path.exists() {
        legacy_path
    } else {
        base.join(current)
    }
}

pub fn data_root() -> Option<&'static PathBuf> {
    DATA_DIR.as_ref()
}

pub fn app_data_dir<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<PathBuf> {
    match data_root() {
        Some(path) => Ok(path.clone()),
        None => app.path().app_data_dir(),
    }
}

pub fn store_path(name: &str) -> PathBuf {
    match data_root() {
        Some(path) => path.join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_pre_rename_folders_keep_being_used() {
        let base = std::env::temp_dir().join(format!("ru-meet-dirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        assert_eq!(product_dir(base.clone()), base.join("Ru-Meet"));
        assert_eq!(recordings_dir(base.clone()), base.join("ru-meet-recordings"));

        std::fs::create_dir(base.join("Meetily")).unwrap();
        std::fs::create_dir(base.join("meetily-recordings")).unwrap();
        assert_eq!(product_dir(base.clone()), base.join("Meetily"));
        assert_eq!(recordings_dir(base.clone()), base.join("meetily-recordings"));

        std::fs::remove_dir_all(&base).unwrap();
    }
}
