//! Automatic recording activity is confirmed by Silero, never by volume alone.
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const SIGNAL_THRESHOLD: f32 = 0.005;
const SPEECH_PROBABILITY: f32 = 0.6;
const MIN_SPEECH_FRAMES: u32 = 7; // Seven 32ms frames, approximately 224ms.
const MAX_FRAME_GAP: u32 = 3;
const ACTIVITY_HOLD: Duration = Duration::from_secs(6);

#[derive(Default)]
struct SourceActivity {
    speech_frames: u32,
    gap_frames: u32,
    last_frame: Option<Instant>,
    last_active: Option<Instant>,
}

impl SourceActivity {
    fn observe(&mut self, rms: f32, probability: f32, now: Instant) {
        if self
            .last_frame
            .is_some_and(|last| now.saturating_duration_since(last) > Duration::from_millis(300))
        {
            self.speech_frames = 0;
            self.gap_frames = 0;
        }
        self.last_frame = Some(now);
        if rms.is_finite()
            && probability.is_finite()
            && rms >= SIGNAL_THRESHOLD
            && probability >= SPEECH_PROBABILITY
        {
            self.speech_frames = self.speech_frames.saturating_add(1);
            self.gap_frames = 0;
            if self.speech_frames >= MIN_SPEECH_FRAMES {
                self.last_active = Some(now);
            }
        } else {
            self.gap_frames = self.gap_frames.saturating_add(1);
            if self.gap_frames > MAX_FRAME_GAP {
                self.speech_frames = 0;
            }
        }
    }

    fn is_active(&self, now: Instant) -> bool {
        self.last_active
            .is_some_and(|last| now.saturating_duration_since(last) <= ACTIVITY_HOLD)
    }
}

#[derive(Default)]
struct SpeechActivity {
    generation: u64,
    microphone: SourceActivity,
    output: SourceActivity,
}

static ACTIVITY: LazyLock<Mutex<SpeechActivity>> =
    LazyLock::new(|| Mutex::new(SpeechActivity::default()));

pub(crate) fn observe(
    rms: f32,
    probability: f32,
    microphone: bool,
    generation: u64,
    captured_at: Instant,
) {
    let mut activity = ACTIVITY.lock().unwrap_or_else(|error| error.into_inner());
    if activity.generation != generation {
        return;
    }
    let source = if microphone {
        &mut activity.microphone
    } else {
        &mut activity.output
    };
    source.observe(rms, probability, captured_at);
}

pub(crate) fn confirmed_sources() -> (bool, bool) {
    let activity = ACTIVITY.lock().unwrap_or_else(|error| error.into_inner());
    let now = Instant::now();
    (
        activity.microphone.is_active(now),
        activity.output.is_active(now),
    )
}

/// Whether each source had confirmed speech within `window` — unlike
/// `confirmed_sources`, without the `ACTIVITY_HOLD` tail.
pub(crate) fn speech_within(window: Duration) -> (bool, bool) {
    let activity = ACTIVITY.lock().unwrap_or_else(|error| error.into_inner());
    let now = Instant::now();
    let recent = |source: &SourceActivity| {
        source
            .last_active
            .is_some_and(|last| now.saturating_duration_since(last) <= window)
    };
    (recent(&activity.microphone), recent(&activity.output))
}

pub(crate) fn can_start(playback: bool, microphone_speech: bool, output_speech: bool) -> bool {
    microphone_speech || (playback && output_speech)
}

pub(crate) fn reset(generation: u64) {
    *ACTIVITY.lock().unwrap_or_else(|error| error.into_inner()) = SpeechActivity {
        generation,
        ..Default::default()
    };
}

pub(crate) fn clear_if_current(generation: u64) {
    let mut activity = ACTIVITY.lock().unwrap_or_else(|error| error.into_inner());
    if activity.generation == generation {
        activity.microphone = SourceActivity::default();
        activity.output = SourceActivity::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loud_non_speech_never_triggers() {
        let now = Instant::now();
        let mut source = SourceActivity::default();
        for _ in 0..100 {
            source.observe(0.3, 0.1, now);
        }
        assert!(!source.is_active(now));
    }

    #[test]
    fn noise_and_single_positive_frame_are_rejected() {
        let now = Instant::now();
        let mut source = SourceActivity::default();
        for _ in 0..100 {
            source.observe(0.001, 0.9, now);
        }
        source.observe(0.2, 0.9, now);
        assert!(!source.is_active(now));
    }

    #[test]
    fn sustained_vad_confirmed_speech_triggers_and_expires() {
        let now = Instant::now();
        let mut source = SourceActivity::default();
        for _ in 0..MIN_SPEECH_FRAMES {
            source.observe(0.02, 0.9, now);
        }
        assert!(source.is_active(now + Duration::from_secs(4)));
        assert!(!source.is_active(now + Duration::from_secs(7)));
    }

    #[test]
    fn separated_clicks_do_not_accumulate_into_speech() {
        let now = Instant::now();
        let mut source = SourceActivity::default();
        for _ in 0..30 {
            source.observe(0.2, 0.9, now);
            for _ in 0..4 {
                source.observe(0.0, 0.0, now);
            }
        }
        assert!(!source.is_active(now));
    }

    #[test]
    fn an_open_playback_stream_cannot_bypass_vad() {
        assert!(!can_start(true, false, false));
        assert!(can_start(true, false, true));
        assert!(!can_start(false, false, true));
        assert!(can_start(false, true, false));
    }

    #[test]
    fn capture_gaps_require_fresh_sustained_speech() {
        let now = Instant::now();
        let mut source = SourceActivity::default();
        for _ in 0..MIN_SPEECH_FRAMES {
            source.observe(0.02, 0.9, now);
        }
        source.observe(0.02, 0.9, now + Duration::from_secs(10));
        assert!(!source.is_active(now + Duration::from_secs(10)));
    }

    #[test]
    fn switching_devices_rejects_stale_vad_results() {
        reset(10);
        let now = Instant::now();
        for _ in 0..MIN_SPEECH_FRAMES {
            observe(0.02, 0.9, true, 10, now);
        }
        assert_eq!(confirmed_sources(), (true, false));
        reset(11);
        for _ in 0..MIN_SPEECH_FRAMES {
            observe(0.02, 0.9, true, 10, now);
        }
        assert_eq!(confirmed_sources(), (false, false));
        clear_if_current(10);
        assert_eq!(ACTIVITY.lock().unwrap().generation, 11);
        reset(12);
    }
}
