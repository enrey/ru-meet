use crate::audio::recording_session::{SessionPhase, RECORDING_SESSION};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const START_DEBOUNCE: Duration = Duration::from_secs(2);
const END_GRACE: Duration = Duration::from_secs(30);
/// After a manual stop (or a manual recording) the detector stays paused until
/// nobody has spoken for this long, so the stopped conversation is not
/// re-recorded right away. Measured from the last recognised speech frame.
const REARM_SILENCE: Duration = Duration::from_secs(5);

/// Whether the detector is currently paused after a manual stop; read by
/// `get_automation_preferences` so the UI does not claim to be listening.
static PAUSED_AFTER_MANUAL_STOP: AtomicBool = AtomicBool::new(false);

pub(crate) fn paused_after_manual_stop() -> bool {
    PAUSED_AFTER_MANUAL_STOP.load(Ordering::Relaxed)
}

/// Set while the in-app meeting player plays; see `set_meeting_playback_active`.
static MEETING_PLAYBACK_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_meeting_playback_active(active: bool) {
    if MEETING_PLAYBACK_ACTIVE.swap(active, Ordering::Relaxed) != active {
        log::info!("In-app meeting playback {}", if active { "started" } else { "stopped" });
    }
}

#[derive(Default)]
struct Detection {
    first_seen: Option<Instant>,
    absent_since: Option<Instant>,
    /// Set by a manual stop or a running manual recording; cleared by silence.
    paused_since: Option<Instant>,
}

impl Detection {
    fn observe(&mut self, active: bool, now: Instant) {
        if active {
            self.first_seen.get_or_insert(now);
            self.absent_since = None;
        } else {
            self.absent_since.get_or_insert(now);
            if self.ended(now) {
                self.first_seen = None;
            }
        }
    }
    fn pause(&mut self, now: Instant) {
        self.paused_since = Some(now);
    }
    /// Lift the pause once `REARM_SILENCE` has passed since it began with no
    /// speech in that window. Speech that continues the stopped conversation
    /// keeps it paused; the next utterance after the silence starts recording.
    fn rearm_if_silent(&mut self, speech_within_rearm_window: bool, now: Instant) {
        if self.paused_since.is_some_and(|since| now.duration_since(since) >= REARM_SILENCE)
            && !speech_within_rearm_window
        {
            self.paused_since = None;
            self.first_seen = None;
        }
    }
    fn paused(&self) -> bool {
        self.paused_since.is_some()
    }
    fn can_start(&self, now: Instant) -> bool {
        !self.paused()
            && self.absent_since.is_none()
            && self
                .first_seen
                .is_some_and(|time| now.duration_since(time) >= START_DEBOUNCE)
    }
    fn ended(&self, now: Instant) -> bool {
        self.absent_since
            .is_some_and(|time| now.duration_since(time) >= END_GRACE)
    }
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    #[cfg(target_os = "windows")]
    tauri::async_runtime::spawn(async move {
        let mut detection = Detection::default();
        let mut preview = super::preview_keeper::PreviewKeeper::default();
        let mut owned: Option<(u64, Instant)> = None;
        let mut last_error = None;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let settings = app
                .state::<super::AutomationState>()
                .0
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some((generation, _)) = owned {
                if !RECORDING_SESSION.is_automatic_generation(generation) {
                    owned = None;
                    // The user stopped our recording by hand.
                    detection.pause(Instant::now());
                }
            }
            // Speech detection must not depend on which page (if any) is open.
            preview.tick(&app, settings.auto_record_meetings).await;
            let (microphone_speech, output_speech) = super::microphone_activity::confirmed_sources();
            let mutes = crate::audio::recording_sources::current();
            let microphone_speech = microphone_speech && !mutes.microphone;
            let output_speech = output_speech && !mutes.system;
            let active = if settings.auto_record_meetings {
                match tokio::task::spawn_blocking(move || {
                    super::windows::has_active_playback(&settings.excluded_apps)
                })
                .await
                {
                    Ok(Ok(active)) => {
                        last_error = None;
                        if owned.is_some() {
                            // Keep an established meeting across natural speech pauses.
                            (active && !mutes.system) || microphone_speech
                        } else {
                            super::microphone_activity::can_start(active, microphone_speech, output_speech)
                        }
                    }
                    result => {
                        let error = format!("Audio stream detection failed: {result:?}");
                        if last_error.as_ref() != Some(&error) {
                            log::warn!("{error}");
                            last_error = Some(error);
                        }
                        // An enumeration failure is not evidence that a meeting ended.
                        if microphone_speech {
                            true
                        } else {
                            continue;
                        }
                    }
                }
            } else {
                false
            };
            // Our own playback never starts a recording; one already running
            // is left to end on its own signals.
            let active = active && (owned.is_some() || !MEETING_PLAYBACK_ACTIVE.load(Ordering::Relaxed));
            let now = Instant::now();
            detection.observe(active, now);
            let (recent_microphone, recent_output) =
                super::microphone_activity::speech_within(REARM_SILENCE);
            detection.rearm_if_silent(
                (recent_microphone && !mutes.microphone) || (recent_output && !mutes.system),
                now,
            );
            let paused = owned.is_none()
                && !RECORDING_SESSION.is_active()
                && detection.paused()
                && settings.auto_record_meetings;
            if PAUSED_AFTER_MANUAL_STOP.swap(paused, Ordering::Relaxed) != paused {
                log::info!("Automatic recording detector {}", if paused { "paused until 5s of silence" } else { "listening" });
                let _ = app.emit("auto-record-meetings-changed", settings.auto_record_meetings);
            }
            crate::tray::refresh_tray_indicator(&app);
            if let Some((generation, started)) = owned {
                let disabled = !app
                    .state::<super::AutomationState>()
                    .0
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .auto_record_meetings;
                if disabled || detection.ended(now) {
                    let end = detection.absent_since.unwrap_or(now);
                    let duration = end.saturating_duration_since(started).as_secs_f64();
                    match RECORDING_SESSION
                        .stop_automatic(app.clone(), generation, duration)
                        .await
                    {
                        Ok(result) => {
                            if result.is_some() {
                                let state = app.state::<crate::notifications::commands::NotificationManagerState<R>>();
                                let _ = crate::notifications::commands::show_recording_stopped_notification(&app, &state).await;
                            }
                        }
                        Err(error) => {
                            log::error!("Automatic recording stop failed: {error}");
                            let _ = app.emit("automatic-recording-error", &error);
                        }
                    }
                    owned = None;
                    detection = Detection::default();
                }
            } else if detection.can_start(now) && RECORDING_SESSION.phase() == SessionPhase::Idle {
                // Do not capture before the user's first-run setup is complete.
                if !crate::onboarding::load_onboarding_status(&app)
                    .await
                    .map(|s| s.completed)
                    .unwrap_or(false)
                    || app.try_state::<crate::state::AppState>().is_none()
                {
                    continue;
                }
                if !app
                    .state::<super::AutomationState>()
                    .0
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .auto_record_meetings
                {
                    continue;
                }
                let title = format!(
                    "Auto Meeting {} {}",
                    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"),
                    uuid::Uuid::new_v4()
                );
                match RECORDING_SESSION
                    .start_automatic(app.clone(), title.clone())
                    .await
                {
                    Ok(generation) => {
                        owned = Some((generation, detection.first_seen.unwrap_or(now)));
                        let state = app
                            .state::<crate::notifications::commands::NotificationManagerState<R>>();
                        let _ =
                            crate::notifications::commands::show_recording_started_notification(
                                &app,
                                &state,
                                Some(title),
                            )
                            .await;
                    }
                    Err(error) => {
                        log::warn!("Automatic recording could not start: {error}");
                        // Retry after the same silence instead of every tick.
                        detection.pause(now);
                        let _ = app.emit("automatic-recording-error", &error);
                    }
                }
            } else if RECORDING_SESSION.is_active() && owned.is_none() {
                // A manual recording owns the session; after it stops the
                // detector waits for silence like after a manual stop.
                detection.pause(now);
            }
        }
    });
    #[cfg(not(target_os = "windows"))]
    let _ = app;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pauses_shorter_than_thirty_seconds_do_not_stop_a_meeting() {
        let start = Instant::now();
        let mut detector = Detection::default();
        detector.observe(true, start);
        detector.observe(false, start + Duration::from_secs(3));
        assert!(!detector.ended(start + Duration::from_secs(32)));
        assert!(detector.ended(start + Duration::from_secs(33)));
    }
    #[test]
    fn brief_streams_do_not_start_and_silence_has_a_grace_period() {
        let start = Instant::now();
        let mut detector = Detection::default();
        detector.observe(true, start);
        assert!(!detector.can_start(start));
        assert!(detector.can_start(start + START_DEBOUNCE));
        detector.observe(false, start + Duration::from_secs(3));
        assert!(!detector.ended(start + Duration::from_secs(10)));
        detector.observe(true, start + Duration::from_secs(11));
        assert!(!detector.ended(start + Duration::from_secs(30)));
    }
    #[test]
    fn manual_stop_waits_for_five_seconds_of_silence() {
        let start = Instant::now();
        let mut detector = Detection::default();
        detector.observe(true, start);
        detector.pause(start);
        // Ongoing speech keeps the detector paused.
        detector.rearm_if_silent(true, start + Duration::from_secs(4));
        detector.rearm_if_silent(true, start + Duration::from_secs(60));
        assert!(!detector.can_start(start + Duration::from_secs(60)));
        // Five silent seconds re-arm it; new speech starts after the debounce.
        detector.observe(false, start + Duration::from_secs(62));
        detector.rearm_if_silent(false, start + Duration::from_secs(66));
        assert!(!detector.paused());
        detector.observe(true, start + Duration::from_secs(68));
        assert!(!detector.can_start(start + Duration::from_secs(68)));
        assert!(detector.can_start(start + Duration::from_secs(70)));
    }
    #[test]
    fn pause_lasts_at_least_five_seconds_even_in_silence() {
        let start = Instant::now();
        let mut detector = Detection::default();
        detector.pause(start);
        detector.rearm_if_silent(false, start + Duration::from_secs(2));
        assert!(detector.paused());
        detector.rearm_if_silent(false, start + REARM_SILENCE);
        assert!(!detector.paused());
    }
}
