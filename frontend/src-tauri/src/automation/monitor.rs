use crate::audio::recording_session::{SessionPhase, RECORDING_SESSION};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const START_DEBOUNCE: Duration = Duration::from_secs(2);
const END_GRACE: Duration = Duration::from_secs(15);

#[derive(Default)]
struct Detection {
    first_seen: Option<Instant>,
    absent_since: Option<Instant>,
    suppressed: bool,
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
                self.suppressed = false;
            }
        }
    }
    fn can_start(&self, now: Instant) -> bool {
        !self.suppressed
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
                    detection.suppressed = true;
                }
            }
            let microphone_active =
                settings.auto_record_meetings && super::microphone_activity::is_active();
            let active = if settings.auto_record_meetings {
                match tokio::task::spawn_blocking(move || {
                    super::windows::has_active_audio(&settings.excluded_apps)
                })
                .await
                {
                    Ok(Ok(active)) => {
                        last_error = None;
                        active || microphone_active
                    }
                    result => {
                        let error = format!("Audio stream detection failed: {result:?}");
                        if last_error.as_ref() != Some(&error) {
                            log::warn!("{error}");
                            last_error = Some(error);
                        }
                        // An enumeration failure is not evidence that a meeting ended.
                        if microphone_active {
                            true
                        } else {
                            continue;
                        }
                    }
                }
            } else {
                false
            };
            let now = Instant::now();
            detection.observe(active, now);
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
                detection.suppressed = true;
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
                        let _ = app.emit("automatic-recording-error", &error);
                    }
                }
            } else if RECORDING_SESSION.is_active() && owned.is_none() && active {
                // Manual recordings and manual stops take precedence for the
                // entire current stream; wait for its end before trying again.
                detection.suppressed = true;
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
    fn manual_stop_suppresses_restart_until_stream_ends() {
        let start = Instant::now();
        let mut detector = Detection::default();
        detector.observe(true, start);
        detector.suppressed = true;
        assert!(!detector.can_start(start + Duration::from_secs(60)));
        detector.observe(false, start + Duration::from_secs(61));
        detector.observe(false, start + Duration::from_secs(80));
        detector.observe(true, start + Duration::from_secs(81));
        assert!(detector.can_start(start + Duration::from_secs(84)));
    }
}
