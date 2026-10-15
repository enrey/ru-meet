//! Speech heard by the running recording, as marked by the transcription VAD
//! in `audio/pipeline.rs` — no second Silero runs for it. Automatic recording
//! uses it to end a meeting, for the silence backstop and to discard
//! recordings that never contained speech.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
/// Set once the VAD has judged any audio of this recording. Without it a
/// broken VAD would look like a recording without speech.
static MEASURED: AtomicBool = AtomicBool::new(false);
static SPEECH_MS: AtomicU64 = AtomicU64::new(0);
/// Milliseconds since `EPOCH` of the last window with speech; 0 = none yet.
static LAST_SPEECH_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordingSpeech {
    pub measured: bool,
    pub total: Duration,
    pub last_speech: Option<Instant>,
}

/// Called when a recording starts.
pub(crate) fn reset() {
    LazyLock::force(&EPOCH);
    MEASURED.store(false, Ordering::Relaxed);
    SPEECH_MS.store(0, Ordering::Relaxed);
    LAST_SPEECH_MS.store(0, Ordering::Relaxed);
}

/// One mixed window of `window` length went through the VAD. Hot path: atomics only.
pub(crate) fn note(window: Duration, speaking: bool) {
    MEASURED.store(true, Ordering::Relaxed);
    if speaking {
        SPEECH_MS.fetch_add(window.as_millis() as u64, Ordering::Relaxed);
        let now = EPOCH.elapsed().as_millis().max(1) as u64;
        LAST_SPEECH_MS.store(now, Ordering::Relaxed);
    }
}

pub(crate) fn snapshot() -> RecordingSpeech {
    let last = LAST_SPEECH_MS.load(Ordering::Relaxed);
    RecordingSpeech {
        measured: MEASURED.load(Ordering::Relaxed),
        total: Duration::from_millis(SPEECH_MS.load(Ordering::Relaxed)),
        last_speech: (last != 0).then(|| *EPOCH + Duration::from_millis(last)),
    }
}

impl RecordingSpeech {
    /// How long the recording has heard no speech, counting from `floor` at
    /// the earliest. `None` while the VAD has judged nothing: that says
    /// nothing about silence.
    pub(crate) fn quiet_for(&self, floor: Instant, now: Instant) -> Option<Duration> {
        if !self.measured {
            return None;
        }
        let from = self.last_speech.map_or(floor, |last| last.max(floor));
        Some(now.saturating_duration_since(from))
    }

    pub(crate) fn speech_within(&self, window: Duration, now: Instant) -> bool {
        self.last_speech
            .is_some_and(|last| now.saturating_duration_since(last) <= window)
    }
}
