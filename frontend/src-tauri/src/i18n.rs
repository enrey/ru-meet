//! Interface language for text the Rust core shows on its own: the tray menu
//! and system notifications. The webview owns the choice and reports it via
//! `set_ui_language`; until it does, Russian is assumed, matching the clean
//! install default of the UI.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Runtime};

static RUSSIAN: AtomicBool = AtomicBool::new(true);

pub fn is_russian() -> bool {
    RUSSIAN.load(Ordering::Relaxed)
}

/// Pick the string for the current interface language.
pub fn tr(en: &'static str, ru: &'static str) -> &'static str {
    if is_russian() {
        ru
    } else {
        en
    }
}

#[tauri::command]
pub fn set_ui_language<R: Runtime>(app: AppHandle<R>, language: String) {
    let russian = language != "en";
    if RUSSIAN.swap(russian, Ordering::Relaxed) != russian {
        crate::tray::update_tray_menu(&app);
    }
}
