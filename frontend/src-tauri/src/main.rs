#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use log;

fn main() {
    // ONNX Runtime logs every graph transform at INFO, which is hundreds of
    // lines per session and drowns out the application's own log.
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info,ort::logging=warn");
    }
    app_lib::logging::init();
    app_lib::run_background_worker();

    // Async logger will be initialized lazily when first needed (after Tauri runtime starts)
    log::info!("Starting application...");
    app_lib::run();
}
