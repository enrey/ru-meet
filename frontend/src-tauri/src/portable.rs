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
        let executable = std::env::current_exe().expect("Cannot locate Meetily executable");
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
