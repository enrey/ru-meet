//! Offline text-to-speech for reading a meeting summary aloud.
//!
//! Speech comes from Qwen3-TTS (Apache-2.0) run through llama.cpp on the GPU;
//! see `qwen.rs` for why it is a separate process. Models and the runtime are
//! installed by hand for now, under `<app data>/models/tts/qwen/<variant>/`
//! and `<app data>/tts-runtime/`.

pub mod commands;
mod qwen;
mod text;

pub use commands::{
    __cmd__tts_get_status, __cmd__tts_set_settings, __cmd__tts_speak, __cmd__tts_stop,
    __tauri_command_name_tts_get_status, __tauri_command_name_tts_set_settings,
    __tauri_command_name_tts_speak, __tauri_command_name_tts_stop, load_settings, preload,
    tts_get_status, tts_set_settings, tts_speak, tts_stop,
};
