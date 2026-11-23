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
    ensure_transcription_model_available, get_or_init_batch_engine,
    get_or_init_transcription_engine, unload_configured_engine, ProviderId,
    TranscriptionEngine,
};
pub use provider::{TranscriptResult, TranscriptionError, TranscriptionProvider};
pub use worker::{reset_speech_detected_flag, start_transcription_task, TranscriptUpdate};

pub(crate) use engine::worker_config;
