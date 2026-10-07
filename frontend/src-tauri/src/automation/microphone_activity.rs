//! Microphone signal activity, independent of other applications' audio sessions.
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

// Approximately -46 dBFS; reject faint noise and require more than a click.
const SIGNAL_THRESHOLD: f32 = 0.005;
const MIN_SIGNAL: Duration = Duration::from_millis(200);
const MAX_SIGNAL_GAP: Duration = Duration::from_millis(300);
// Bridge syllable gaps and the automation monitor's two-second polling interval.
const ACTIVITY_HOLD: Duration = Duration::from_secs(4);

#[derive(Default)]
struct MicrophoneActivity {
    generation: u64,
    signal_since: Option<Instant>,
    last_signal: Option<Instant>,
    last_active: Option<Instant>,
}

impl MicrophoneActivity {
    fn observe(&mut self, rms: f32, now: Instant) {
        if !rms.is_finite() || rms < SIGNAL_THRESHOLD {
            return;
        }
        if self
            .last_signal
            .is_none_or(|last| now.duration_since(last) > MAX_SIGNAL_GAP)
        {
            self.signal_since = Some(now);
        }
        self.last_signal = Some(now);
        if self
            .signal_since
            .is_some_and(|start| now.duration_since(start) >= MIN_SIGNAL)
        {
            self.last_active = Some(now);
        }
    }

    fn is_active(&self, now: Instant) -> bool {
        self.last_active
            .is_some_and(|last| now.duration_since(last) <= ACTIVITY_HOLD)
    }
}

static ACTIVITY: LazyLock<Mutex<MicrophoneActivity>> =
    LazyLock::new(|| Mutex::new(MicrophoneActivity::default()));

pub(crate) fn observe(rms: f32, generation: u64) {
    let mut activity = ACTIVITY.lock().unwrap_or_else(|error| error.into_inner());
    if activity.generation == generation {
        activity.observe(rms, Instant::now());
    }
}

pub(crate) fn is_active() -> bool {
    ACTIVITY
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_active(Instant::now())
}

pub(crate) fn reset(generation: u64) {
    *ACTIVITY.lock().unwrap_or_else(|error| error.into_inner()) = MicrophoneActivity {
        generation,
        ..Default::default()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_noise_and_single_click_do_not_trigger() {
        let start = Instant::now();
        let mut activity = MicrophoneActivity::default();
        for tick in 0..30 {
            activity.observe(0.001, start + Duration::from_millis(tick * 100));
        }
        activity.observe(0.1, start + Duration::from_secs(3));
        assert!(!activity.is_active(start + Duration::from_secs(3)));
    }

    #[test]
    fn microphone_signal_is_detected_without_an_external_audio_session() {
        let start = Instant::now();
        let mut activity = MicrophoneActivity::default();
        for tick in 0..6 {
            activity.observe(0.02, start + Duration::from_millis(tick * 100));
        }
        assert!(activity.is_active(start + Duration::from_secs(2)));
        assert!(activity.is_active(start + Duration::from_secs(4)));
        assert!(!activity.is_active(start + Duration::from_secs(5)));
    }

    #[test]
    fn isolated_clicks_do_not_accumulate_into_speech() {
        let start = Instant::now();
        let mut activity = MicrophoneActivity::default();
        for tick in 0..10 {
            let now = start + Duration::from_secs(tick);
            activity.observe(0.1, now);
            assert!(!activity.is_active(now));
        }
    }

    #[test]
    fn short_syllable_gaps_preserve_activity() {
        let start = Instant::now();
        let mut activity = MicrophoneActivity::default();
        activity.observe(0.02, start);
        activity.observe(0.0, start + Duration::from_millis(100));
        activity.observe(0.02, start + Duration::from_millis(250));
        assert!(activity.is_active(start + Duration::from_millis(250)));
    }

    #[test]
    fn switching_devices_clears_signal_and_ignores_old_meter_updates() {
        reset(10);
        let now = Instant::now();
        {
            let mut activity = ACTIVITY.lock().unwrap();
            activity.observe(0.02, now - Duration::from_millis(300));
            activity.observe(0.02, now);
        }
        assert!(is_active());
        reset(11);
        observe(0.02, 10);
        assert!(ACTIVITY.lock().unwrap().last_signal.is_none());
        assert!(!is_active());
        observe(0.02, 11);
        assert!(ACTIVITY.lock().unwrap().last_signal.is_some());
        reset(12);
    }
}
