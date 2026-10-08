use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

use crate::i18n::tr;
use crate::tray_badge::TrayIndicator;

const PRODUCT_NAME: &str = "Ru-Meet";

/// Last indicator pushed to the tray, so periodic refreshes are free.
static LAST_INDICATOR: std::sync::Mutex<Option<TrayIndicator>> = std::sync::Mutex::new(None);

#[derive(Debug, Clone)]
pub enum RecordingState {
    Stopped,
    Starting,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Stopping,
}

pub fn create_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // Start with default menu, will update with actual state after initialization
    // Pass can_record=true initially, will be updated by update_tray_menu immediately
    let menu = build_menu(app, RecordingState::Stopped, true)?;

    TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip(PRODUCT_NAME)
        .icon(app.default_window_icon().unwrap().clone())
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .build(app)?;

    // Update tray menu with actual recording state after creation
    update_tray_menu(app);

    Ok(())
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, item_id: &str) {
    match item_id {
        "toggle_recording" => toggle_recording_handler(app),
        "pause_recording" => pause_recording_handler(app),
        "resume_recording" => resume_recording_handler(app),
        "stop_recording" => stop_recording_handler(app),
        "open_window" => focus_main_window(app),
        "settings" => {
            focus_main_window(app);
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval("window.location.assign('/settings')");
            }
        }
        "check_updates" => check_updates_handler(app),
        "quit" => app.exit(0),
        _ => {}
    }
}
fn toggle_recording_handler<R: Runtime>(app: &AppHandle<R>) {
    focus_main_window(app);
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        if crate::is_recording().await {
            // Immediately show stopping state
            set_tray_state(&app_clone, RecordingState::Stopping);

            log::info!("Tray toggle: Stopping recording...");

            let stop_result =
                crate::audio::recording_commands::stop_recording(app_clone.clone()).await;

            // Handle result
            match stop_result {
                Ok(_) => {
                    log::info!("Tray toggle: Recording stopped successfully");
                }
                Err(e) => {
                    log::error!("Tray toggle: Failed to stop recording: {}", e);
                    // Revert tray state on error
                    update_tray_menu_async(&app_clone).await;
                }
            }
        } else {
            // Immediately show starting state
            set_tray_state(&app_clone, RecordingState::Starting);

            log::info!("Emitting start recording event from tray");
            if let Some(window) = app_clone.get_webview_window("main") {
                let _ = window.eval("sessionStorage.setItem('autoStartRecording', 'true')"); // Set the flag to start recording automatically
                let _ = window.eval("window.location.assign('/')");
            }
        }
    });
}

fn pause_recording_handler<R: Runtime>(app: &AppHandle<R>) {
    // Immediately show pausing state
    set_tray_state(app, RecordingState::Pausing);

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::audio::recording_commands::pause_recording(app_clone.clone()).await {
            log::error!("Failed to pause recording from tray: {}", e);
            // Revert to current state on error
            update_tray_menu_async(&app_clone).await;
        } else {
            log::info!("Recording paused from tray");
            // The pause_recording function will call update_tray_menu, so no need to call it here
        }
    });
}

fn resume_recording_handler<R: Runtime>(app: &AppHandle<R>) {
    // Immediately show resuming state
    set_tray_state(app, RecordingState::Resuming);

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::audio::recording_commands::resume_recording(app_clone.clone()).await
        {
            log::error!("Failed to resume recording from tray: {}", e);
            // Revert to current state on error
            update_tray_menu_async(&app_clone).await;
        } else {
            log::info!("Recording resumed from tray");
            // The resume_recording function will call update_tray_menu, so no need to call it here
        }
    });
}

fn stop_recording_handler<R: Runtime>(app: &AppHandle<R>) {
    // Immediately show stopping state
    set_tray_state(app, RecordingState::Stopping);

    focus_main_window(app);
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        log::info!("Tray: Stopping recording...");

        let stop_result = crate::audio::recording_commands::stop_recording(app_clone.clone()).await;

        // Handle result
        match stop_result {
            Ok(_) => {
                log::info!("Tray: Recording stopped successfully");
            }
            Err(e) => {
                log::error!("Tray: Failed to stop recording: {}", e);
                // Revert tray state on error
                update_tray_menu_async(&app_clone).await;
            }
        }
    });
}

fn check_updates_handler<R: Runtime>(app: &AppHandle<R>) {
    focus_main_window(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval("window.dispatchEvent(new CustomEvent('check-updates-from-tray'))");
    }
}

pub fn update_tray_menu<R: Runtime>(app: &AppHandle<R>) {
    // For sync update, spawn async task to get current state
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        // Small delay to ensure recording state has been updated
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        update_tray_menu_async(&app_clone).await;
    });
}

pub fn set_tray_state<R: Runtime>(app: &AppHandle<R>, state: RecordingState) {
    log::info!("Tray: Setting intermediate state: {:?}", state);
    // During recording state transitions, we assume recording is allowed (we're already recording)
    if let Ok(menu) = build_menu(app, state, true) {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let result = tray.set_menu(Some(menu));
            log::info!("Tray: Intermediate state menu update result: {:?}", result);
        } else {
            log::warn!("Tray: Could not find tray with id 'main-tray'");
        }
    } else {
        log::error!("Tray: Failed to build menu for intermediate state");
    }
}

async fn get_current_recording_state() -> RecordingState {
    // Check if currently recording
    let is_recording = crate::audio::recording_commands::is_recording().await;
    log::info!(
        "Tray: get_current_recording_state - is_recording: {}",
        is_recording
    );

    if !is_recording {
        log::info!("Tray: Recording state is Stopped");
        return RecordingState::Stopped;
    }

    // Check if paused
    let is_paused = crate::audio::recording_commands::is_recording_paused().await;
    log::info!("Tray: is_paused: {}", is_paused);

    if is_paused {
        log::info!("Tray: Recording state is Paused");
        RecordingState::Paused
    } else {
        log::info!("Tray: Recording state is Recording");
        RecordingState::Recording
    }
}

/// Check if recording is allowed based on onboarding status and transcription model availability
/// Returns true if:
/// - Onboarding is complete (user may prefer Whisper later), OR
/// - Parakeet transcription model is ready (downloaded)
async fn check_can_record<R: Runtime>(app: &AppHandle<R>) -> bool {
    // First check if onboarding is complete
    let onboarding_complete = match crate::onboarding::load_onboarding_status(app).await {
        Ok(status) => status.completed,
        Err(e) => {
            log::warn!(
                "Tray: Failed to load onboarding status: {}, assuming complete",
                e
            );
            true // Assume complete if we can't check (safe default)
        }
    };

    // If onboarding is complete, always allow recording
    // (user may prefer Whisper or have their own transcription setup)
    if onboarding_complete {
        return true;
    }

    // During onboarding, check if Parakeet transcription model is ready
    match crate::parakeet_engine::commands::parakeet_has_available_models().await {
        Ok(has_models) => has_models,
        Err(e) => {
            log::warn!(
                "Tray: Failed to check Parakeet models: {}, assuming not ready",
                e
            );
            false
        }
    }
}

/// Recompute the tray badge and tooltip from the recording session and the
/// automatic-recording detector. Cheap when nothing changed.
pub fn refresh_tray_indicator<R: Runtime>(app: &AppHandle<R>) {
    use crate::audio::recording_session::{SessionPhase, RECORDING_SESSION};
    let indicator = match RECORDING_SESSION.phase() {
        SessionPhase::Paused => TrayIndicator::Paused,
        SessionPhase::Starting | SessionPhase::Recording => TrayIndicator::Recording,
        // Stopping only finalises (transcripts, diarization): capture is over.
        SessionPhase::Idle | SessionPhase::Stopping if crate::automation::is_listening(app) => {
            TrayIndicator::Listening
        }
        SessionPhase::Idle | SessionPhase::Stopping => TrayIndicator::Idle,
    };

    {
        let mut last = LAST_INDICATOR.lock().unwrap_or_else(|e| e.into_inner());
        if *last == Some(indicator) {
            return;
        }
        *last = Some(indicator);
    }

    if let Some(base) = app.default_window_icon() {
        let rgba = crate::tray_badge::compose(base.rgba(), base.width(), base.height(), indicator);
        let side = crate::tray_badge::SIZE;
        let icon = tauri::image::Image::new_owned(rgba, side, side);
        // The window icon is what the Windows taskbar button shows; keep it in
        // step with the tray. (No-op on macOS, where windows have no icon.)
        for window in app.webview_windows().values() {
            let _ = window.set_icon(icon.clone());
        }
        if let Some(tray) = app.tray_by_id("main-tray") {
            let _ = tray.set_icon(Some(icon));
        }
    }
    let Some(tray) = app.tray_by_id("main-tray") else {
        return;
    };
    let status = match indicator {
        TrayIndicator::Idle => None,
        TrayIndicator::Listening => Some(tr("automatic recording is waiting for speech", "автозапись ждёт речь")),
        TrayIndicator::Recording => Some(tr("recording", "идёт запись")),
        TrayIndicator::Paused => Some(tr("recording paused", "запись на паузе")),
    };
    let tooltip = status.map_or_else(|| PRODUCT_NAME.to_string(), |status| format!("{PRODUCT_NAME} — {status}"));
    let _ = tray.set_tooltip(Some(tooltip));
}

pub async fn update_tray_menu_async<R: Runtime>(app: &AppHandle<R>) {
    refresh_tray_indicator(app);
    log::info!("Tray: update_tray_menu_async called");
    // Get the current recording state
    let recording_state = get_current_recording_state().await;
    log::info!("Tray: Current recording state: {:?}", recording_state);

    // Determine if recording should be allowed
    // Only block recording during incomplete onboarding when no transcription model is ready
    let can_record = check_can_record(app).await;
    log::info!("Tray: can_record: {}", can_record);

    if let Ok(menu) = build_menu(app, recording_state, can_record) {
        if let Some(tray) = app.tray_by_id("main-tray") {
            let result = tray.set_menu(Some(menu));
            log::info!("Tray: Menu update result: {:?}", result);
        } else {
            log::warn!("Tray: Could not find tray with id 'main-tray'");
        }
    } else {
        log::error!("Tray: Failed to build menu");
    }
}

fn build_menu<R: Runtime>(
    app: &AppHandle<R>,
    state: RecordingState,
    can_record: bool, // True if recording is allowed (onboarding complete OR transcription model ready)
) -> tauri::Result<tauri::menu::Menu<R>> {
    let mut builder = MenuBuilder::new(app);

    // If recording is not allowed (during onboarding, no transcription model), show disabled message
    if !can_record {
        builder = builder.item(
            &MenuItemBuilder::new(tr("⏳ Downloading transcription model...", "⏳ Загрузка модели транскрипции..."))
                .enabled(false)
                .build(app)?,
        );
    } else {
        match state {
            RecordingState::Stopped => {
                builder = builder.item(
                    &MenuItemBuilder::with_id("toggle_recording", tr("Start Recording", "Начать запись")).build(app)?,
                );
            }
            RecordingState::Starting => {
                builder = builder.item(
                    &MenuItemBuilder::new(tr("🔄 Starting Recording...", "🔄 Запуск записи..."))
                        .enabled(false)
                        .build(app)?,
                );
            }
            RecordingState::Recording => {
                builder = builder
                    .item(
                        &MenuItemBuilder::with_id("pause_recording", tr("⏸ Pause Recording", "⏸ Приостановить запись"))
                            .build(app)?,
                    )
                    .item(
                        &MenuItemBuilder::with_id("stop_recording", tr("⏹ Stop Recording", "⏹ Остановить запись"))
                            .build(app)?,
                    );
            }
            RecordingState::Pausing => {
                builder = builder
                    .item(
                        &MenuItemBuilder::new(tr("⏸ Pausing...", "⏸ Приостановка..."))
                            .enabled(false)
                            .build(app)?,
                    )
                    .item(
                        &MenuItemBuilder::with_id("stop_recording", tr("⏹ Stop Recording", "⏹ Остановить запись"))
                            .build(app)?,
                    );
            }
            RecordingState::Paused => {
                builder = builder
                    .item(
                        &MenuItemBuilder::with_id("resume_recording", tr("▶ Resume Recording", "▶ Возобновить запись"))
                            .build(app)?,
                    )
                    .item(
                        &MenuItemBuilder::with_id("stop_recording", tr("⏹ Stop Recording", "⏹ Остановить запись"))
                            .build(app)?,
                    );
            }
            RecordingState::Resuming => {
                builder = builder
                    .item(
                        &MenuItemBuilder::new(tr("▶ Resuming...", "▶ Возобновление..."))
                            .enabled(false)
                            .build(app)?,
                    )
                    .item(
                        &MenuItemBuilder::with_id("stop_recording", tr("⏹ Stop Recording", "⏹ Остановить запись"))
                            .build(app)?,
                    );
            }
            RecordingState::Stopping => {
                builder = builder.item(
                    &MenuItemBuilder::new(tr("⏹ Stopping...", "⏹ Остановка..."))
                        .enabled(false)
                        .build(app)?,
                );
            }
        }
    }

    builder
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("open_window", tr("Open Main Window", "Открыть главное окно")).build(app)?)
        .item(&MenuItemBuilder::with_id("settings", tr("Settings", "Настройки")).build(app)?)
        .item(&MenuItemBuilder::with_id("check_updates", tr("Check for Updates", "Проверить обновления")).build(app)?)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("quit", tr("Quit", "Выход")).build(app)?)
        .build()
}

pub(crate) fn focus_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.unminimize() {
            log::error!("Failed to unminimize main window: {}", e);
        }

        if let Err(e) = window.show() {
            log::error!("Failed to show main window: {}", e);
        }

        if let Err(e) = window.set_focus() {
            log::error!("Failed to focus main window: {}", e);
        }

        if let Err(e) = window.eval("window.focus()") {
            log::error!("Failed to focus main webview: {}", e);
        }
    } else {
        log::warn!("Could not find main window");
    }
}
