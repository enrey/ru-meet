//! Bounded PCM queue: ONNX inference never runs on a CPAL callback.
use crate::audio::vad::ContinuousVadProcessor;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant};

pub(crate) struct PreviewAudio {
    pub device: String,
    pub microphone: bool,
    pub sample_rate: u32,
    pub samples: Vec<f32>,
    pub captured_at: Instant,
}

pub(crate) fn start(generation: u64) -> std::io::Result<SyncSender<PreviewAudio>> {
    let (sender, receiver) = sync_channel::<PreviewAudio>(32);
    std::thread::Builder::new().name("automatic-recording-vad".into()).spawn(move || {
        let mut processors = HashMap::<(String, u32), ContinuousVadProcessor>::new();
        let mut failed = HashSet::new();
        while crate::audio::level_monitor::is_current_generation(generation) {
            let packet = match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(packet) => packet,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            if !crate::audio::level_monitor::is_current_generation(generation) { break; }
            // Never turn delayed/stale PCM into a fresh recording trigger.
            if packet.captured_at.elapsed() > Duration::from_secs(1) {
                processors.clear();
                super::microphone_activity::clear_if_current(generation);
                continue;
            }
            let key = (packet.device.clone(), packet.sample_rate);
            if failed.contains(&key) { continue; }
            if !processors.contains_key(&key) {
                match ContinuousVadProcessor::new(packet.sample_rate, 500) {
                    Ok(processor) => { processors.insert(key.clone(), processor); }
                    Err(error) => {
                        log::warn!("Automatic recording VAD unavailable for {}: {error}; speech trigger disabled", packet.device);
                        failed.insert(key.clone());
                        continue;
                    }
                }
            }
            match processors.get_mut(&key).unwrap().process_activity(&packet.samples) {
                Ok(frames) => {
                    for (rms, probability) in frames {
                        super::microphone_activity::observe(rms, probability, packet.microphone, generation, packet.captured_at);
                    }
                }
                Err(error) => {
                    log::warn!("Automatic recording VAD failed for {}: {error}; speech trigger disabled", packet.device);
                    processors.remove(&key);
                    failed.insert(key);
                }
            }
        }
    })?;
    Ok(sender)
}
