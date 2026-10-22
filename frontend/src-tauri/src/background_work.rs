//! Admission for expensive work. Recording reserves the machine before capture
//! starts and keeps it until finalization finishes. Reservation requests
//! interruption immediately and never waits for background computation.

use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct State {
    recordings: usize,
    active: Option<CancellationToken>,
    stop: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Default)]
pub(crate) struct Scheduler {
    state: Mutex<State>,
    changed: Notify,
}

static SCHEDULER: LazyLock<Arc<Scheduler>> = LazyLock::new(|| Arc::new(Scheduler::default()));

pub(crate) struct RecordingPriority(Arc<Scheduler>);

pub(crate) struct BackgroundPermit {
    scheduler: Arc<Scheduler>,
    interrupted: CancellationToken,
}

impl Scheduler {
    pub(crate) fn reserve_recording(self: &Arc<Self>) -> RecordingPriority {
        let stop = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.recordings += 1;
            if let Some(active) = &state.active {
                active.cancel();
            }
            state.stop.clone()
        };
        // Only sends the OS termination request. Reaping is asynchronous and
        // never delays opening the recording streams.
        if let Some(stop) = stop {
            stop();
        }
        RecordingPriority(self.clone())
    }

    async fn acquire(self: &Arc<Self>) -> BackgroundPermit {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.recordings == 0 && state.active.is_none() {
                    let interrupted = CancellationToken::new();
                    state.active = Some(interrupted.clone());
                    return BackgroundPermit {
                        scheduler: self.clone(),
                        interrupted,
                    };
                }
            }
            changed.await;
        }
    }
}

impl Drop for RecordingPriority {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recordings -= 1;
        log::info!("Recording reservation released; background work may resume");
        self.0.changed.notify_waiters();
    }
}

impl BackgroundPermit {
    pub(crate) fn stop_on_recording(&self, stop: Arc<dyn Fn() + Send + Sync>) {
        let interrupted = {
            let mut state = self
                .scheduler
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.stop = Some(stop.clone());
            self.interrupted.is_cancelled()
        };
        // Covers recording winning the race between admission and child spawn.
        if interrupted {
            stop();
        }
    }

    pub(crate) fn interruption(&self) -> CancellationToken {
        self.interrupted.clone()
    }
}

impl Drop for BackgroundPermit {
    fn drop(&mut self) {
        {
            let mut state = self
                .scheduler
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.active = None;
            state.stop = None;
        }
        self.scheduler.changed.notify_waiters();
    }
}

pub(crate) fn recording_status() -> (bool, bool) {
    let state = SCHEDULER
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    (
        state.recordings > 0,
        false, // Recording never waits for background cleanup.
    )
}

pub(crate) fn reserve_recording() -> RecordingPriority {
    log::info!("Pausing background work for recording");
    SCHEDULER.reserve_recording()
}

pub(crate) async fn acquire() -> BackgroundPermit {
    SCHEDULER.acquire().await
}

/// Interrupt one request, wait for its cleanup, then retry it after recording.
/// Completed requests/chunks remain the caller's responsibility and are kept.
pub(crate) async fn restartable<T, F, Fut>(
    cancellation: Option<&CancellationToken>,
    operation: F,
) -> Result<T, String>
where
    F: FnMut(CancellationToken) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    restartable_on(&SCHEDULER, cancellation, operation).await
}

async fn restartable_on<T, F, Fut>(
    scheduler: &Arc<Scheduler>,
    cancellation: Option<&CancellationToken>,
    mut operation: F,
) -> Result<T, String>
where
    F: FnMut(CancellationToken) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    loop {
        let permit = tokio::select! {
            biased;
            _ = async {
                match cancellation {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            } => return Err("Summary generation was cancelled".into()),
            permit = scheduler.acquire() => permit,
        };
        let interruption = permit.interruption();
        let attempt_token = cancellation
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let attempt = operation(attempt_token.clone());
        tokio::pin!(attempt);
        let result = tokio::select! {
            biased;
            _ = interruption.cancelled() => {
                log::info!("Background request paused for recording");
                attempt_token.cancel();
                // Keep background admission until the child has exited; recording
                // itself does not wait for this cleanup.
                let _ = attempt.await;
                None
            }
            result = &mut attempt => Some(result),
        };
        drop(permit);
        if let Some(result) = result {
            return result;
        }
    }
}

/// Cancellation remains responsive while a job waits behind a recording.
pub(crate) async fn acquire_while(continue_work: impl Fn() -> bool) -> Option<BackgroundPermit> {
    acquire_while_on(&SCHEDULER, continue_work).await
}

pub(crate) async fn acquire_while_on(
    scheduler: &Arc<Scheduler>,
    continue_work: impl Fn() -> bool,
) -> Option<BackgroundPermit> {
    let admission = scheduler.acquire();
    tokio::pin!(admission);
    loop {
        if !continue_work() {
            return None;
        }
        tokio::select! {
            permit = &mut admission => return continue_work().then_some(permit),
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
    }
}

pub(crate) async fn acquire_batch(
    cancelled: &'static std::sync::atomic::AtomicBool,
) -> anyhow::Result<BackgroundPermit> {
    acquire_while(|| !cancelled.load(std::sync::atomic::Ordering::SeqCst))
        .await
        .ok_or_else(|| anyhow::anyhow!("Background processing cancelled"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn recording_interrupts_active_work_and_blocks_queued_work_until_released() {
        let scheduler = Arc::new(Scheduler::default());
        let active = scheduler.acquire().await;
        let recording = scheduler.reserve_recording();
        assert!(active.interruption().is_cancelled());
        drop(active);
        assert!(timeout(Duration::from_millis(10), scheduler.acquire())
            .await
            .is_err());
        drop(recording);
        let resumed = timeout(Duration::from_secs(1), scheduler.acquire())
            .await
            .unwrap();
        assert!(!resumed.interruption().is_cancelled());
    }

    #[tokio::test]
    async fn expensive_jobs_are_serial_and_failed_start_releases_reservation() {
        let scheduler = Arc::new(Scheduler::default());
        let first = scheduler.acquire().await;
        assert!(timeout(Duration::from_millis(10), scheduler.acquire())
            .await
            .is_err());
        drop(first);
        let failed_start = scheduler.reserve_recording();
        let other_reservation = scheduler.reserve_recording();
        drop(failed_start);
        assert!(timeout(Duration::from_millis(10), scheduler.acquire())
            .await
            .is_err());
        drop(other_reservation);
        let _resumed = timeout(Duration::from_secs(1), scheduler.acquire())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn recording_signals_process_stop_without_waiting_for_its_exit() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let scheduler = Arc::new(Scheduler::default());
        let active = scheduler.acquire().await;
        let signalled = Arc::new(AtomicBool::new(false));
        let flag = signalled.clone();
        active.stop_on_recording(Arc::new(move || {
            flag.store(true, Ordering::SeqCst);
        }));
        let recording = scheduler.reserve_recording();
        assert!(signalled.load(Ordering::SeqCst));
        assert!(active.interruption().is_cancelled());
        // The old permit may remain alive during OS cleanup; capture need not wait.
        drop(recording);
        assert!(timeout(Duration::from_millis(10), scheduler.acquire())
            .await
            .is_err());
        drop(active);
        let _resumed = timeout(Duration::from_secs(1), scheduler.acquire())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn interrupted_request_cleans_up_before_capture_and_retries_after_recording() {
        let scheduler = Arc::new(Scheduler::default());
        let worker_scheduler = scheduler.clone();
        let (started, mut starts) = tokio::sync::mpsc::unbounded_channel();
        let (cleanup, cleanup_done) = tokio::sync::oneshot::channel();
        let mut cleanup = Some(cleanup);
        let worker = tokio::spawn(async move {
            let mut attempts = 0;
            restartable_on(&worker_scheduler, None, move |token| {
                attempts += 1;
                let attempt = attempts;
                let started = started.clone();
                let cleanup = cleanup.take();
                async move {
                    started.send(attempt).unwrap();
                    if attempt == 1 {
                        token.cancelled().await;
                        cleanup.unwrap().send(()).unwrap();
                        Err("interrupted".into())
                    } else {
                        Ok("completed chunk")
                    }
                }
            })
            .await
        });
        assert_eq!(starts.recv().await, Some(1));
        let recording = scheduler.reserve_recording();
        cleanup_done.await.unwrap();
        assert!(starts.try_recv().is_err());
        drop(recording);
        assert_eq!(
            timeout(Duration::from_secs(1), worker)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "completed chunk"
        );
        assert_eq!(starts.recv().await, Some(2));
    }

    #[tokio::test]
    async fn cancelling_a_paused_request_does_not_run_it_after_recording() {
        let scheduler = Arc::new(Scheduler::default());
        let recording = scheduler.reserve_recording();
        let cancelled = CancellationToken::new();
        let cancellation = cancelled.clone();
        let worker_scheduler = scheduler.clone();
        let worker = tokio::spawn(async move {
            restartable_on(&worker_scheduler, Some(&cancellation), |_| async {
                panic!("a cancelled queued request must never run");
                #[allow(unreachable_code)]
                Ok::<_, String>(())
            })
            .await
        });
        cancelled.cancel();
        assert_eq!(
            timeout(Duration::from_secs(1), worker)
                .await
                .unwrap()
                .unwrap(),
            Err("Summary generation was cancelled".into())
        );
        drop(recording);
        let _permit = timeout(Duration::from_secs(1), scheduler.acquire())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn batch_or_speech_cancellation_during_recording_does_not_wait_for_stop() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let scheduler = Arc::new(Scheduler::default());
        let _recording = scheduler.reserve_recording();
        let keep_running = Arc::new(AtomicBool::new(true));
        let worker_flag = keep_running.clone();
        let worker_scheduler = scheduler.clone();
        let worker = tokio::spawn(async move {
            acquire_while_on(&worker_scheduler, || worker_flag.load(Ordering::SeqCst))
                .await
                .is_none()
        });
        tokio::task::yield_now().await;
        keep_running.store(false, Ordering::SeqCst);
        assert!(timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap());
    }
}

pub(crate) fn scheduler() -> Arc<Scheduler> {
    SCHEDULER.clone()
}
