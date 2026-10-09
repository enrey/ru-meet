//! Offline text-to-speech for reading a meeting summary aloud.
//!
//! The reading is prepared ahead of time into a file next to the meeting; see
//! `summary_audio.rs`.
//!
//! Speech comes from Qwen3-TTS (Apache-2.0) run through llama.cpp on the GPU;
//! see `qwen.rs` for why it is a separate process. Models and the runtime are
//! installed by hand for now, under `<app data>/models/tts/qwen/<variant>/`
//! and `<app data>/tts-runtime/`.

pub mod commands;
mod qwen;
pub mod summary_audio;
mod text;

pub use commands::{
    __cmd__tts_get_status, __cmd__tts_set_settings, __tauri_command_name_tts_get_status,
    __tauri_command_name_tts_set_settings, load_settings, preload, tts_get_status,
    tts_set_settings,
};
pub use summary_audio::{
    __cmd__tts_prepare_summary_audio, __cmd__tts_summary_audio,
    __tauri_command_name_tts_prepare_summary_audio, __tauri_command_name_tts_summary_audio,
    prepare_if_automatic, tts_prepare_summary_audio, tts_summary_audio,
};
