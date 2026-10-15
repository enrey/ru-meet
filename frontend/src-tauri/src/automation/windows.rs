//! Enumerates active WASAPI audio sessions. Capture sessions say which
//! processes are recording from a microphone (the call signal, ADR 0004);
//! render sessions feed the in-person mode, where playback alone never starts
//! a recording without confirmed VAD speech.
use super::call_apps::{AudioClient, Identifier};
use std::collections::{HashMap, HashSet};
use windows::core::{Interface, PWSTR};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Media::Audio::{
        eCapture, eRender, AudioSessionStateActive, EDataFlow, IAudioSessionControl2,
        IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    },
    System::{
        Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED},
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

/// How far up the process tree a capture session is attributed: far enough
/// for `msedgewebview2.exe` → `msedgewebview2.exe` → `ms-teams.exe`.
const ANCESTOR_DEPTH: usize = 4;

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct ProcessEntry {
    parent: u32,
    /// Lower-cased executable file name from the snapshot.
    name: String,
}

fn process_table() -> windows::core::Result<HashMap<u32, ProcessEntry>> {
    unsafe {
        let snapshot = OwnedHandle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut table = HashMap::new();
        Process32FirstW(snapshot.0, &mut entry)?;
        loop {
            let length = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            table.insert(
                entry.th32ProcessID,
                ProcessEntry {
                    parent: entry.th32ParentProcessID,
                    name: String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase(),
                },
            );
            if Process32NextW(snapshot.0, &mut entry).is_err() {
                break;
            }
        }
        Ok(table)
    }
}

/// Ru-Meet and every process it started.
fn own_processes(table: &HashMap<u32, ProcessEntry>) -> HashSet<u32> {
    let mut own = HashSet::from([std::process::id()]);
    loop {
        let before = own.len();
        for (&pid, entry) in table {
            if own.contains(&entry.parent) {
                own.insert(pid);
            }
        }
        if own.len() == before {
            break;
        }
    }
    own
}

fn own_executable_name() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().to_lowercase()))
}

fn executable_name(pid: u32) -> windows::core::Result<String> {
    unsafe {
        let process = OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )?;
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        Ok(path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&path)
            .to_lowercase())
    }
}

/// CPAL and other COM users can initialize Tokio's reusable blocking threads
/// with a different apartment type. Use a fresh MTA thread so detection keeps
/// working after device enumeration and recording start.
fn on_mta_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    std::thread::Builder::new()
        .name("meeting-stream-detection".into())
        .spawn(move || unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
            let _apartment = ComApartment;
            work()
        })
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "Audio stream detection thread panicked".to_string())?
}

/// Process ids of active sessions on every active endpoint of `flow`.
/// Must run on an MTA thread.
fn active_session_pids(flow: EDataFlow) -> Result<Vec<u32>, String> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let devices = enumerator
            .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let mut pids = Vec::new();
        for index in 0..devices.GetCount().map_err(|e| e.to_string())? {
            let device = devices.Item(index).map_err(|e| e.to_string())?;
            let manager: IAudioSessionManager2 = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| e.to_string())?;
            let sessions = manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
            for index in 0..sessions.GetCount().map_err(|e| e.to_string())? {
                let control = sessions.GetSession(index).map_err(|e| e.to_string())?;
                if control.GetState().map_err(|e| e.to_string())? != AudioSessionStateActive {
                    continue;
                }
                let control: IAudioSessionControl2 = control.cast().map_err(|e| e.to_string())?;
                let pid = control.GetProcessId().map_err(|e| e.to_string())?;
                if pid != 0 && !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
        Ok(pids)
    }
}

/// Processes other than Ru-Meet capturing from any active input device.
pub fn mic_clients() -> Result<Vec<AudioClient>, String> {
    on_mta_thread(|| {
        let pids = active_session_pids(eCapture)?;
        let table = process_table().map_err(|e| e.to_string())?;
        let own = own_processes(&table);
        let own_name = own_executable_name();
        let mut clients = Vec::new();
        for pid in pids {
            if own.contains(&pid) {
                continue;
            }
            let Some(entry) = table.get(&pid) else {
                continue;
            };
            if own_name.as_deref() == Some(entry.name.as_str()) {
                continue;
            }
            let mut identifiers = vec![Identifier::Executable(entry.name.clone())];
            let mut parent = entry.parent;
            let mut seen = HashSet::from([pid]);
            while identifiers.len() <= ANCESTOR_DEPTH && seen.insert(parent) {
                let Some(ancestor) = table.get(&parent) else {
                    break;
                };
                identifiers.push(Identifier::Executable(ancestor.name.clone()));
                parent = ancestor.parent;
            }
            clients.push(AudioClient {
                display: entry.name.clone(),
                identifiers,
            });
        }
        Ok(clients)
    })
}

/// Whether an app other than Ru-Meet and the exclusions is playing audio.
pub fn has_active_playback(excluded_apps: &[String]) -> Result<bool, String> {
    let excluded_apps = excluded_apps.to_vec();
    on_mta_thread(move || {
        let pids = active_session_pids(eRender)?;
        let table = process_table().map_err(|e| e.to_string())?;
        let own = own_processes(&table);
        // Ru-Meet itself, also when one of its processes is not our descendant.
        let own_name = own_executable_name();
        for pid in pids {
            if own.contains(&pid) {
                continue;
            }
            // Fail closed for inaccessible processes: their exclusion
            // status cannot be verified, so they must not trigger capture.
            let Ok(name) = executable_name(pid) else {
                continue;
            };
            if own_name.as_deref() != Some(name.as_str())
                && !excluded_apps
                    .iter()
                    .any(|excluded| excluded.eq_ignore_ascii_case(&name))
            {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_can_enumerate_live_playback_sessions() {
        has_active_playback(&[]).expect("Windows playback session enumeration must work");
    }
    #[test]
    fn windows_can_enumerate_live_capture_sessions() {
        let clients = mic_clients().expect("Windows capture session enumeration must work");
        let own = own_executable_name();
        assert!(clients.iter().all(|client| Some(&client.display) != own.as_ref()));
    }
}
