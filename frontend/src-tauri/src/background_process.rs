//! Owned process tree. Stop only sends termination; callers reap asynchronously.
use anyhow::{Context, Result};
use std::{
    io::Write,
    process::{Child, ChildStdin, ChildStdout, Command, ExitStatus},
    sync::{Arc, Mutex},
};

pub(crate) struct Process {
    child: Mutex<Option<Child>>,
    input: Mutex<Option<ChildStdin>>,
    group: Group,
}
impl Process {
    pub(crate) fn spawn(command: &mut Command) -> Result<(Arc<Self>, Option<ChildStdout>)> {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000 | 0x0000_4000); // hidden, below normal priority
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .context("Could not start background process")?;
        let group = match Group::attach(&child) {
            Ok(group) => group,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdout = child.stdout.take();
        let input = child.stdin.take();
        Ok((
            Arc::new(Self {
                child: Mutex::new(Some(child)),
                input: Mutex::new(input),
                group,
            }),
            stdout,
        ))
    }
    pub(crate) fn send(&self, input: &[u8]) -> Result<()> {
        // Pipe backpressure must never hold the process-control lock.
        let mut writer = self.input.lock().unwrap_or_else(|e| e.into_inner());
        let stdin = writer.as_mut().context("Worker input is closed")?;
        stdin.write_all(input)?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
    }
    pub(crate) fn stop(&self) {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = slot.as_mut() {
            // Keep the handle alive until the group signal has been sent (no PID reuse).
            self.group.stop();
            let _ = child.kill();
        }
    }
    pub(crate) fn try_wait(&self) -> Result<Option<ExitStatus>> {
        let mut slot = self.child.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_mut()
            .context("Process already reaped")?
            .try_wait()
            .map_err(Into::into)
    }
    pub(crate) async fn stopped(&self) {
        self.stop();
        while matches!(self.try_wait(), Ok(None)) || !self.group.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
        if let Some(mut child) = self
            .child
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            // No wait in a recording-start or async-future drop path.
            let _ = std::thread::Builder::new()
                .name("background-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                });
        }
    }
}

#[cfg(windows)]
struct Group(isize);
#[cfg(windows)]
impl Group {
    fn attach(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::{
            core::PCWSTR,
            Win32::{Foundation::HANDLE, System::JobObjects::*},
        };
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null())?;
            let group = Self(handle.0);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(handle, HANDLE(child.as_raw_handle() as isize))?;
            Ok(group)
        }
    }
    fn stop(&self) {
        unsafe {
            let _ = windows::Win32::System::JobObjects::TerminateJobObject(
                windows::Win32::Foundation::HANDLE(self.0),
                1,
            );
        }
    }
    fn is_empty(&self) -> bool {
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                HANDLE(self.0),
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of_val(&info) as u32,
                None,
            )
            .is_ok()
                && info.ActiveProcesses == 0
        }
    }
}
#[cfg(windows)]
impl Drop for Group {
    fn drop(&mut self) {
        unsafe {
            let _ =
                windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(self.0));
        }
    }
}
#[cfg(unix)]
struct Group(i32);
#[cfg(unix)]
impl Group {
    fn attach(child: &Child) -> Result<Self> {
        Ok(Self(child.id() as i32))
    }
    fn is_empty(&self) -> bool {
        true
    }
    fn stop(&self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}

/// Run one external computation, repeating only attempts preempted by recording.
/// This is called on a blocking worker, never on the recording thread.
pub(crate) fn run_blocking(
    command: &mut Command,
    keep_running: impl Fn() -> bool,
    on_interruption: impl Fn(),
) -> Result<Option<ExitStatus>> {
    loop {
        let Some(permit) =
            tauri::async_runtime::block_on(crate::background_work::acquire_while(&keep_running))
        else {
            return Ok(None);
        };
        let interrupted = permit.interruption();
        let (process, _) = Process::spawn(command)?;
        let stop = process.clone();
        permit.stop_on_recording(Arc::new(move || stop.stop()));
        let mut status = loop {
            if interrupted.is_cancelled() || !keep_running() {
                tauri::async_runtime::block_on(process.stopped());
                on_interruption();
                break None;
            }
            if let Some(status) = process.try_wait()? {
                break Some(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        tauri::async_runtime::block_on(process.stopped());
        if interrupted.is_cancelled() {
            on_interruption();
            status = None;
        }
        drop(permit);
        drop(process);
        if !keep_running() {
            return Ok(None);
        }
        if let Some(status) = status {
            return Ok(Some(status));
        }
    }
}
