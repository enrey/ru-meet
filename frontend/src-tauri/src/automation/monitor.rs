//! The automatic recording loop (ADR 0004).
//!
//! Starts when a call app captures the microphone (or, in the in-person mode,
//! on confirmed speech). Ends when the call app has released the microphone
//! and the recording has heard no speech for `END_GRACE`; two backstops end it
//! regardless of who holds the microphone.
use super::call_apps::{self, MicHolder, MicHolders};
use super::recording_speech::{self, RecordingSpeech};
use crate::audio::recording_session::{SessionPhase, RECORDING_SESSION};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const TICK: Duration = Duration::from_secs(2);
const START_DEBOUNCE: Duration = Duration::from_secs(2);
const END_GRACE: Duration = Duration::from_secs(30);
/// After a manual stop (or a manual recording, a backstop) the detector stays
/// paused until the call app has released the microphone — and, in the
/// in-person mode, nobody has spoken — for this long, so the stopped
/// conversation is not re-recorded right away.
const REARM_SILENCE: Duration = Duration::from_secs(5);
/// Speech the recording heard this recently keeps a meeting going: a tick
/// plus pipeline latency.
const RECORDING_SPEECH_WINDOW: Duration = Duration::from_secs(4);
/// Backstop against an app that never releases the microphone.
const SILENCE_STOP: Duration = Duration::from_secs(10 * 60);
/// Hard ceiling on an automatic recording.
const MAX_DURATION: Duration = Duration::from_secs(5 * 60 * 60);
/// Waits after a refused start: 30 s, 1, 2, 4, 8 min, then every 10 minutes.
const FIRST_RETRY: Duration = Duration::from_secs(30);
const LONGEST_RETRY: Duration = Duration::from_secs(10 * 60);

/// Whether the detector is currently paused until the call ends (after a
/// manual stop, a manual recording or a backstop); read by
/// `get_automation_preferences` so the UI does not claim to be listening.
static PAUSED_UNTIL_CALL_ENDS: AtomicBool = AtomicBool::new(false);

pub(crate) fn paused_until_call_ends() -> bool {
    PAUSED_UNTIL_CALL_ENDS.load(Ordering::Relaxed)
}

/// Whether the last attempt to list microphone clients failed, so the UI
/// does not claim to be waiting for a call it cannot see.
static DETECTION_FAILED: AtomicBool = AtomicBool::new(false);

pub(crate) fn detection_failed() -> bool {
    DETECTION_FAILED.load(Ordering::Relaxed)
}

/// Set while the in-app meeting player plays; see `set_meeting_playback_active`.
static MEETING_PLAYBACK_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_meeting_playback_active(active: bool) {
    if MEETING_PLAYBACK_ACTIVE.swap(active, Ordering::Relaxed) != active {
        log::info!("In-app meeting playback {}", if active { "started" } else { "stopped" });
    }
}

/// Last observed microphone holders, for settings.
static MIC_HOLDERS: Mutex<Vec<MicHolder>> = Mutex::new(Vec::new());

pub(crate) fn mic_holders() -> Vec<MicHolder> {
    MIC_HOLDERS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Returns whether the holders changed.
fn publish_mic_holders(holders: &[MicHolder]) -> bool {
    let mut current = MIC_HOLDERS.lock().unwrap_or_else(|e| e.into_inner());
    if current.as_slice() == holders {
        return false;
    }
    *current = holders.to_vec();
    true
}

#[derive(Default)]
struct Detection {
    first_seen: Option<Instant>,
    absent_since: Option<Instant>,
    /// Set by a manual stop, a running manual recording or a backstop;
    /// cleared once the call is over.
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
    /// Lift the pause once `REARM_SILENCE` has passed since it began and the
    /// call was not `busy` within that window. A call that continues keeps it
    /// paused; the next one after it starts recording.
    fn rearm_if_silent(&mut self, busy: bool, now: Instant) {
        if self.paused_since.is_some_and(|since| now.duration_since(since) >= REARM_SILENCE)
            && !busy
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

/// The automatic recording this loop started.
struct Owned {
    generation: u64,
    /// When the meeting was first detected, for its duration.
    first_seen: Instant,
    started: Instant,
    /// Silence is counted from here at the earliest; moved while paused so a
    /// long pause does not trip the silence backstop on resume.
    quiet_floor: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    CallEnded,
    Disabled,
    Silence,
    MaxDuration,
}

impl StopReason {
    /// After a backstop the call is presumed to still be going.
    fn is_backstop(self) -> bool {
        matches!(self, Self::Silence | Self::MaxDuration)
    }
}

fn backstop(owned: &Owned, speech: &RecordingSpeech, now: Instant) -> Option<StopReason> {
    if now.saturating_duration_since(owned.started) >= MAX_DURATION {
        return Some(StopReason::MaxDuration);
    }
    speech
        .quiet_for(owned.quiet_floor, now)
        .is_some_and(|quiet| quiet >= SILENCE_STOP)
        .then_some(StopReason::Silence)
}

fn retry_delay(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    (FIRST_RETRY * 2u32.pow(doublings)).min(LONGEST_RETRY)
}

/// In-person mode only: an app other than Ru-Meet and the exclusions is playing.
async fn playback_active(excluded_apps: &[String]) -> bool {
    #[cfg(target_os = "windows")]
    {
        let excluded_apps = excluded_apps.to_vec();
        matches!(
            tokio::task::spawn_blocking(move || super::windows::has_active_playback(&excluded_apps))
                .await,
            Ok(Ok(true))
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = excluded_apps;
        false
    }
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    if !super::is_supported() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut detection = Detection::default();
        let mut preview = super::preview_keeper::PreviewKeeper::default();
        let mut owned: Option<Owned> = None;
        let mut call_last_active: Option<Instant> = None;
        let mut start_failures = 0u32;
        let mut retry_at: Option<Instant> = None;
        let mut last_error = None;
        loop {
            tokio::time::sleep(TICK).await;
            let settings = app
                .state::<super::AutomationState>()
                .0
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(current) = &owned {
                if !RECORDING_SESSION.is_automatic_generation(current.generation) {
                    owned = None;
                    // The user stopped our recording by hand.
                    detection.pause(Instant::now());
                }
            }
            let in_person = settings.auto_record_meetings && settings.record_in_person;
            // Speech detection must not depend on which page (if any) is open.
            preview.tick(&app, in_person).await;

            let holders = if settings.auto_record_meetings {
                match tokio::task::spawn_blocking(super::mic_clients).await {
                    Ok(Some(Ok(clients))) => {
                        last_error = None;
                        Some(call_apps::evaluate(
                            &clients,
                            &settings.call_apps,
                            &settings.excluded_apps,
                            settings.record_any_app,
                        ))
                    }
                    result => {
                        let error = format!("Microphone client detection failed: {result:?}");
                        if last_error.as_ref() != Some(&error) {
                            log::warn!("{error}");
                            last_error = Some(error);
                        }
                        None
                    }
                }
            } else {
                Some(MicHolders::default())
            };
            let shown: &[MicHolder] = holders.as_ref().map_or(&[], |h| &h.holders);
            let holders_changed = publish_mic_holders(shown);
            if holders_changed {
                if let Some(holders) = &holders {
                    log::info!("Microphone holders: {:?}", holders.holders);
                }
            }
            let failed = holders.is_none();
            let failure_changed = DETECTION_FAILED.swap(failed, Ordering::Relaxed) != failed;
            if holders_changed || failure_changed {
                let _ = app.emit("auto-record-meetings-changed", settings.auto_record_meetings);
            }

            let now = Instant::now();
            let call_active = holders.as_ref().is_some_and(MicHolders::call_active);
            // An enumeration failure is not evidence that a meeting ended.
            let call_unknown = holders.is_none();
            if call_active {
                call_last_active = Some(now);
            }

            let mutes = crate::audio::recording_sources::current();
            let no_sources = mutes.microphone && mutes.system;
            let (microphone_speech, output_speech) = super::microphone_activity::confirmed_sources();
            let microphone_speech = in_person && microphone_speech && !mutes.microphone;
            let output_speech = in_person && output_speech && !mutes.system;
            let playback = in_person && playback_active(&settings.excluded_apps).await;
            // Our own player never starts a recording in the in-person mode.
            let in_person_start = !MEETING_PLAYBACK_ACTIVE.load(Ordering::Relaxed)
                && super::microphone_activity::can_start(playback, microphone_speech, output_speech);
            let in_person_keep = microphone_speech || (playback && !mutes.system);

            let speech = recording_speech::snapshot();
            let active = if owned.is_some() {
                // Keep an established meeting across natural speech pauses.
                call_active
                    || call_unknown
                    || speech.speech_within(RECORDING_SPEECH_WINDOW, now)
                    || in_person_keep
            } else {
                settings.auto_record_meetings && !no_sources && (call_active || in_person_start)
            };
            detection.observe(active, now);

            let call_busy = call_unknown
                || call_last_active.is_some_and(|last| now.duration_since(last) <= REARM_SILENCE);
            let (recent_microphone, recent_output) =
                super::microphone_activity::speech_within(REARM_SILENCE);
            let in_person_busy = in_person
                && ((recent_microphone && !mutes.microphone) || (recent_output && !mutes.system));
            detection.rearm_if_silent(call_busy || in_person_busy, now);

            let paused = owned.is_none()
                && !RECORDING_SESSION.is_active()
                && detection.paused()
                && settings.auto_record_meetings;
            if PAUSED_UNTIL_CALL_ENDS.swap(paused, Ordering::Relaxed) != paused {
                log::info!(
                    "Automatic recording detector {}",
                    if paused { "paused until the call ends" } else { "listening" }
                );
                let _ = app.emit("auto-record-meetings-changed", settings.auto_record_meetings);
            }
            crate::tray::refresh_tray_indicator(&app);

            if let Some(current) = owned.as_mut() {
                if RECORDING_SESSION.phase() == SessionPhase::Paused {
                    current.quiet_floor = now;
                }
                let reason = if !settings.auto_record_meetings {
                    Some(StopReason::Disabled)
                } else if detection.ended(now) {
                    Some(StopReason::CallEnded)
                } else {
                    backstop(current, &speech, now)
                };
                let Some(reason) = reason else {
                    continue;
                };
                let end = match reason {
                    StopReason::CallEnded => detection.absent_since.unwrap_or(now),
                    _ => now,
                };
                let duration = end.saturating_duration_since(current.first_seen).as_secs_f64();
                log::info!("Stopping automatic recording: {reason:?} after {duration:.0}s");
                match RECORDING_SESSION
                    .stop_automatic(app.clone(), current.generation, duration)
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
                if reason.is_backstop() {
                    // Wait until the app releases the microphone.
                    detection.pause(now);
                }
            } else if detection.can_start(now)
                && RECORDING_SESSION.phase() == SessionPhase::Idle
                && retry_at.map_or(true, |at| now >= at)
            {
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
                log::info!(
                    "Starting automatic recording (call: {call_active}, in-person: {in_person_start})"
                );
                match RECORDING_SESSION
                    .start_automatic(app.clone(), title.clone())
                    .await
                {
                    Ok(generation) => {
                        let started = Instant::now();
                        owned = Some(Owned {
                            generation,
                            first_seen: detection.first_seen.unwrap_or(now),
                            started,
                            quiet_floor: started,
                        });
                        start_failures = 0;
                        retry_at = None;
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
                        start_failures += 1;
                        let wait = retry_delay(start_failures);
                        log::warn!(
                            "Automatic recording could not start: {error}; retrying in {}s",
                            wait.as_secs()
                        );
                        retry_at = Some(now + wait);
                        let _ = app.emit("automatic-recording-error", &error);
                    }
                }
            } else if RECORDING_SESSION.is_active() && owned.is_none() {
                // A manual recording owns the session; after it stops the
                // detector waits for the call to end like after a manual stop.
                detection.pause(now);
            }
            if owned.is_none() && !active {
                // A new call is a new episode: refusals during the last one
                // say nothing about it.
                start_failures = 0;
                retry_at = None;
            }
        }
    });
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
    fn manual_stop_waits_until_the_call_is_over() {
        let start = Instant::now();
        let mut detector = Detection::default();
        detector.observe(true, start);
        detector.pause(start);
        // The call app still holding the microphone keeps the detector paused.
        detector.rearm_if_silent(true, start + Duration::from_secs(4));
        detector.rearm_if_silent(true, start + Duration::from_secs(60));
        assert!(!detector.can_start(start + Duration::from_secs(60)));
        // Five free seconds re-arm it; the next call starts after the debounce.
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

    fn owned(start: Instant) -> Owned {
        Owned {
            generation: 1,
            first_seen: start,
            started: start,
            quiet_floor: start,
        }
    }

    fn speech(measured: bool, last: Option<Instant>) -> RecordingSpeech {
        RecordingSpeech {
            measured,
            total: Duration::ZERO,
            last_speech: last,
        }
    }

    #[test]
    fn ten_silent_minutes_end_a_call_that_never_releases_the_microphone() {
        let start = Instant::now();
        let recording = owned(start);
        let spoke = Some(start + Duration::from_secs(60));
        assert_eq!(backstop(&recording, &speech(true, spoke), start + Duration::from_secs(60) + SILENCE_STOP - Duration::from_secs(1)), None);
        assert_eq!(
            backstop(&recording, &speech(true, spoke), start + Duration::from_secs(60) + SILENCE_STOP),
            Some(StopReason::Silence)
        );
        // Silence from the very start counts too.
        assert_eq!(backstop(&recording, &speech(true, None), start + SILENCE_STOP), Some(StopReason::Silence));
    }
    #[test]
    fn an_unmeasured_recording_is_not_silent_and_pauses_do_not_count() {
        let start = Instant::now();
        assert_eq!(backstop(&owned(start), &speech(false, None), start + SILENCE_STOP * 2), None);
        let mut recording = owned(start);
        recording.quiet_floor = start + Duration::from_secs(30 * 60);
        assert_eq!(backstop(&recording, &speech(true, None), start + Duration::from_secs(35 * 60)), None);
    }
    #[test]
    fn the_duration_ceiling_applies_even_during_speech() {
        let start = Instant::now();
        let now = start + MAX_DURATION;
        assert_eq!(backstop(&owned(start), &speech(true, Some(now)), now), Some(StopReason::MaxDuration));
        assert!(StopReason::MaxDuration.is_backstop());
        assert!(!StopReason::CallEnded.is_backstop());
    }
    #[test]
    fn refused_starts_back_off_up_to_ten_minutes() {
        let delays: Vec<u64> = (1..=7).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(delays, vec![30, 60, 120, 240, 480, 600, 600]);
        assert_eq!(retry_delay(u32::MAX), LONGEST_RETRY);
    }
}
