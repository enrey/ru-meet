//! Keeps the speech-detection preview running while automatic recording is on.
//!
//! The preview (level monitor + Silero VAD) used to exist only while the
//! "capture signal" panel was mounted on the home page, so automatic recording
//! went deaf on every other page and in the tray. The panel still drives its own
//! preview for meters and device switching; this keeper only fills the gaps.
use std::time::{Duration, Instant};
use tauri::{AppHandle, Runtime};

/// After a failed or empty start, don't hammer device enumeration every tick.
const RETRY_AFTER: Duration = Duration::from_secs(10);
/// Re-resolve devices periodically so a new default microphone is picked up.
const DEVICE_RECHECK: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct PreviewKeeper {
    /// Generation of the preview this keeper started, while it is still current.
    owned: Option<(u64, Vec<String>)>,
    next_attempt: Option<Instant>,
    checked_at: Option<Instant>,
}

impl PreviewKeeper {
    pub(crate) async fn tick<R: Runtime>(&mut self, app: &AppHandle<R>, wanted: bool) {
        use crate::audio::level_monitor;

        if let Some((generation, _)) = &self.owned {
            if !level_monitor::is_current_generation(*generation) {
                // The panel took over (or stopped) the preview; it is not ours anymore.
                self.owned = None;
            }
        }

        if !wanted {
            if let Some((generation, _)) = self.owned.take() {
                if level_monitor::is_current_generation(generation) {
                    let _ = level_monitor::stop_monitoring();
                }
            }
            return;
        }

        let now = Instant::now();
        if level_monitor::is_monitoring() {
            // Someone keeps the preview alive. Only our own one needs refreshing.
            let due = self
                .checked_at
                .map_or(true, |at| now.duration_since(at) >= DEVICE_RECHECK);
            if let (Some((_, devices)), true) = (&self.owned, due) {
                self.checked_at = Some(now);
                let current = preview_devices(app).await;
                if !current.is_empty() && &current != devices {
                    log::info!("Automatic recording preview devices changed: {devices:?} -> {current:?}");
                    self.start(app, current, now);
                }
            }
            return;
        }

        if self.next_attempt.is_some_and(|at| now < at) {
            return;
        }
        let devices = preview_devices(app).await;
        if devices.is_empty() {
            self.next_attempt = Some(now + RETRY_AFTER);
            return;
        }
        log::info!("Automatic recording: starting background speech detection on {devices:?}");
        self.start(app, devices, now);
    }

    fn start<R: Runtime>(&mut self, app: &AppHandle<R>, devices: Vec<String>, now: Instant) {
        self.checked_at = Some(now);
        match crate::audio::level_monitor::start_monitoring_thread(app.clone(), devices.clone()) {
            Ok(generation) => {
                self.owned = Some((generation, devices));
                self.next_attempt = None;
            }
            Err(error) => {
                log::warn!("Automatic recording preview could not start: {error}");
                self.owned = None;
                self.next_attempt = Some(now + RETRY_AFTER);
            }
        }
    }
}

/// The microphone and system output an automatic recording would capture:
/// the preferred devices when they exist, otherwise the system defaults.
/// Unlike the recording-start resolvers this never emits UI events.
/// Sources switched off in the capture panel are left out.
async fn preview_devices<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use crate::audio::{default_input_device, default_output_device, parse_audio_device};

    let preferences = crate::audio::recording_preferences::load_recording_preferences(app)
        .await
        .ok();
    let preferred = |pick: fn(&crate::audio::RecordingPreferences) -> Option<&String>| {
        preferences
            .as_ref()
            .and_then(pick)
            .and_then(|name| parse_audio_device(name).ok())
            .map(|device| device.name)
    };

    let input_names: Vec<String> = cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default();
    let microphone = preferred(|p| p.preferred_mic_device.as_ref())
        .filter(|name| input_names.contains(name))
        .or_else(|| default_input_device().ok().map(|device| device.name));
    let system = preferred(|p| p.preferred_system_device.as_ref())
        .or_else(|| default_output_device().ok().map(|device| device.name));

    // A source switched off in the capture panel is not opened at all.
    let mutes = crate::audio::recording_sources::current();
    microphone
        .filter(|_| !mutes.microphone)
        .into_iter()
        .chain(system.filter(|_| !mutes.system))
        .collect()
}
