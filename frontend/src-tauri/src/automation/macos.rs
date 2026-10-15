//! Asks Core Audio which processes are capturing from an input device
//! (`kAudioHardwarePropertyProcessObjectList` + `kAudioProcessPropertyIsRunningInput`,
//! macOS 14.4+). Reading it needs no TCC permission and opens no device.
//! The answer is per process, so our own capture is never mistaken for a call.
use super::call_apps::{AudioClient, Identifier};
use cidre::core_audio as ca;

/// Tauri `identifier`; matches Ru-Meet when a helper of ours is not our pid.
const OWN_BUNDLE_ID: &str = "com.meetily.ai";

/// Processes other than Ru-Meet capturing from any input device. An error
/// (older macOS, Core Audio refusing) means meeting detection is unavailable.
pub fn mic_clients() -> Result<Vec<AudioClient>, String> {
    let own_pid = std::process::id() as i32;
    let processes = ca::System::processes()
        .map_err(|error| format!("Core Audio process list is unavailable: {error:?}"))?;
    let mut clients = Vec::new();
    for process in processes {
        if !process.is_running_input().unwrap_or(false) {
            continue;
        }
        let Ok(pid) = process.pid() else {
            continue;
        };
        if pid == own_pid {
            continue;
        }
        let bundle_id = process
            .bundle_id()
            .ok()
            .map(|id| id.to_string())
            .filter(|id| !id.is_empty());
        if bundle_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(OWN_BUNDLE_ID))
        {
            continue;
        }
        clients.push(AudioClient {
            display: bundle_id.clone().unwrap_or_else(|| format!("pid {pid}")),
            identifiers: bundle_id.map(Identifier::BundleId).into_iter().collect(),
        });
    }
    Ok(clients)
}

#[cfg(test)]
mod tests {
    #[test]
    fn macos_can_list_live_audio_processes() {
        super::mic_clients().expect("Core Audio process list must work on macOS 14.4+");
    }
}
