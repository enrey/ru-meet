// audio/transcription/mod.rs
//
// Transcription module: Provider abstraction, engine management, and worker pool.

mod engine;
mod parakeet_provider;
pub mod provider;
mod whisper_provider;
pub mod worker;

// Re-export commonly used types
pub use engine::{
    get_or_init_batch_engine, get_or_init_transcription_engine, unload_batch_engine,
    unload_configured_engine, ensure_transcription_model_available, ProviderId, TranscriptionEngine,
};
pub use provider::{TranscriptResult, TranscriptionError, TranscriptionProvider};
pub use worker::{reset_speech_detected_flag, start_transcription_task, TranscriptUpdate};
