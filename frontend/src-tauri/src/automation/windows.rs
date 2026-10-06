//! Enumerates playback stream state, not volume. Silence and muting do not
//! end a meeting while its application still has an active audio stream.
use std::collections::{HashMap, HashSet};
use windows::core::{Interface, PWSTR};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Media::Audio::{
        eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
        IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
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

fn own_processes() -> windows::core::Result<HashSet<u32>> {
    unsafe {
        let snapshot = OwnedHandle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut parents = HashMap::new();
        Process32FirstW(snapshot.0, &mut entry)?;
        loop {
            parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            if Process32NextW(snapshot.0, &mut entry).is_err() {
                break;
            }
        }
        let mut own = HashSet::from([std::process::id()]);
        loop {
            let before = own.len();
            for (&pid, parent) in &parents {
                if own.contains(parent) {
                    own.insert(pid);
                }
            }
            if own.len() == before {
                break;
            }
        }
        Ok(own)
    }
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

pub fn has_active_playback(excluded_apps: &[String]) -> Result<bool, String> {
    // CPAL and other COM users can initialize Tokio's reusable blocking
    // threads with a different apartment type. Use a fresh MTA thread so
    // detection keeps working after device enumeration and recording start.
    let excluded_apps = excluded_apps.to_vec();
    std::thread::Builder::new()
        .name("meeting-stream-detection".into())
        .spawn(move || enumerate_playback(&excluded_apps))
        .map_err(|error| error.to_string())?
        .join()
        .map_err(|_| "Audio stream detection thread panicked".to_string())?
}

fn enumerate_playback(excluded_apps: &[String]) -> Result<bool, String> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let _apartment = ComApartment;
        let own = own_processes().map_err(|e| e.to_string())?;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
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
                if pid == 0 || own.contains(&pid) {
                    continue;
                }
                // Fail closed for inaccessible processes: their exclusion
                // status cannot be verified, so they must not trigger capture.
                let Ok(name) = executable_name(pid) else {
                    continue;
                };
                if !excluded_apps
                    .iter()
                    .any(|excluded| excluded.eq_ignore_ascii_case(&name))
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_can_enumerate_live_playback_sessions() {
        has_active_playback(&[]).expect("Windows playback session enumeration must work");
    }
}
