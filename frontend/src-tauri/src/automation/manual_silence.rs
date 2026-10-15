//! Stops a manual recording that has heard no speech for the configured time:
//! people forget to press Stop after a meeting. Automatic recordings have
//! their own end rules in `monitor`, so they are never touched here.
use super::recording_speech::{self, RecordingSpeech};
use crate::audio::recording_session::{SessionPhase, RECORDING_SESSION};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

const TICK: Duration = Duration::from_secs(5);

struct Watch {
    generation: u64,
    /// Silence is counted from here at the earliest; moved while paused so a
    /// long pause does not stop the recording right after resume.
    quiet_floor: Instant,
}

fn silence_reached(speech: &RecordingSpeech, watch: &Watch, now: Instant, limit: Duration) -> bool {
    speech
        .quiet_for(watch.quiet_floor, now)
        .is_some_and(|quiet| quiet >= limit)
}

pub(super) fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let mut watch: Option<Watch> = None;
        loop {
            tokio::time::sleep(TICK).await;
            let now = Instant::now();
            let Some(generation) = RECORDING_SESSION.manual_generation() else {
                watch = None;
                continue;
            };
            let current = match &mut watch {
                Some(current) if current.generation == generation => current,
                _ => watch.insert(Watch {
                    generation,
                    quiet_floor: now,
                }),
            };
            if RECORDING_SESSION.phase() == SessionPhase::Paused {
                current.quiet_floor = now;
                continue;
            }
            let (enabled, minutes) = {
                let state = app.state::<super::AutomationState>();
                let settings = state.0.read().unwrap_or_else(|e| e.into_inner());
                (settings.manual_silence_stop, settings.manual_silence_minutes)
            };
            let limit = Duration::from_secs(u64::from(minutes) * 60);
            if !enabled || !silence_reached(&recording_speech::snapshot(), current, now, limit) {
                continue;
            }
            log::info!("Stopping manual recording after {minutes} min without speech");
            watch = None;
            match RECORDING_SESSION.stop_manual(app.clone(), generation).await {
                Ok(Some(_)) => {
                    let _ = app.emit("recording-stopped-for-silence", minutes);
                    let state = app
                        .state::<crate::notifications::commands::NotificationManagerState<R>>();
                    let _ = crate::notifications::commands::show_recording_stopped_notification(
                        &app, &state,
                    )
                    .await;
                }
                Ok(None) => {}
                Err(error) => log::error!("Stopping a silent manual recording failed: {error}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: Duration = Duration::from_secs(30 * 60);

    fn speech(measured: bool, last_speech: Option<Instant>) -> RecordingSpeech {
        RecordingSpeech {
            measured,
            total: Duration::ZERO,
            last_speech,
        }
    }

    #[test]
    fn stops_only_after_the_limit_without_speech() {
        let start = Instant::now();
        let watch = Watch { generation: 1, quiet_floor: start };
        let quiet = speech(true, None);
        assert!(!silence_reached(&quiet, &watch, start + LIMIT - TICK, LIMIT));
        assert!(silence_reached(&quiet, &watch, start + LIMIT, LIMIT));
        let spoke = speech(true, Some(start + Duration::from_secs(600)));
        assert!(!silence_reached(&spoke, &watch, start + LIMIT, LIMIT));
        assert!(silence_reached(&spoke, &watch, start + LIMIT + Duration::from_secs(600), LIMIT));
    }

    #[test]
    fn an_unmeasured_recording_or_a_pause_is_not_silence() {
        let start = Instant::now();
        let watch = Watch { generation: 1, quiet_floor: start };
        assert!(!silence_reached(&speech(false, None), &watch, start + LIMIT * 4, LIMIT));
        let resumed = Watch { generation: 1, quiet_floor: start + LIMIT };
        assert!(!silence_reached(&speech(true, None), &resumed, start + LIMIT + TICK, LIMIT));
    }
}
