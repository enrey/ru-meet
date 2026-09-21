use crate::database::manager::DatabaseManager;
use std::sync::Mutex;

pub struct AppState {
    pub db_manager: DatabaseManager,
}

/// Startup can continue without an `AppState` when an existing database cannot
/// be opened. The frontend reads this state to offer a recoverable reset flow
/// instead of aborting the process during Tauri setup.
#[derive(Default)]
pub struct DatabaseStartupStatus {
    error: Mutex<Option<String>>,
}

impl DatabaseStartupStatus {
    pub fn set_error(&self, error: String) {
        if let Ok(mut value) = self.error.lock() {
            *value = Some(error);
        }
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|value| value.clone())
    }

    pub fn clear(&self) {
        if let Ok(mut value) = self.error.lock() {
            *value = None;
        }
    }
}
