//! Queue of atomic meeting jobs with delayed recovery of saved failures. FIFO admission is held for a whole
//! stage; `background_work` supplies the recording-aware safe boundaries inside it.
mod recovery;

pub(crate) use recovery::initialize;

use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    sync::{LazyLock, Mutex},
};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::Notify;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Action {
    Speech {
        meeting_id: String,
    },
    Speakers {
        meeting_id: String,
        audio_path: std::path::PathBuf,
    },
    Summary {
        meeting_id: String,
        text: String,
        provider: String,
        model: String,
        prompt: String,
        template: String,
        language: Option<String>,
    },
    RecoverSummary {
        meeting_id: String,
    },
    Import {
        source: String,
        title: String,
        language: Option<String>,
        model: Option<String>,
        provider: Option<String>,
    },
    Retranscription {
        meeting_id: String,
        folder: String,
        language: Option<String>,
        model: Option<String>,
        provider: Option<String>,
        process_after: bool,
    },
    /// Look for the voice library's people among a meeting's unnamed speakers.
    VoiceMatch {
        meeting_id: String,
    },
}

impl Action {
    fn kind(&self) -> &'static str {
        match self {
            Self::Speech { .. } => "tts",
            Self::Speakers { .. } => "diarization",
            Self::Summary { .. } | Self::RecoverSummary { .. } => "summary",
            Self::Import { .. } => "import",
            Self::Retranscription { .. } => "retranscription",
            Self::VoiceMatch { .. } => "voice-match",
        }
    }
    fn meeting_id(&self) -> Option<&str> {
        match self {
            Self::Speech { meeting_id }
            | Self::Speakers { meeting_id, .. }
            | Self::RecoverSummary { meeting_id }
            | Self::Summary { meeting_id, .. }
            | Self::Retranscription { meeting_id, .. }
            | Self::VoiceMatch { meeting_id } => Some(meeting_id),
            Self::Import { .. } => None,
        }
    }
    fn key(&self) -> String {
        let target = match self {
            Self::Import { source, .. } => source,
            _ => self.meeting_id().unwrap_or_default(),
        };
        format!("{}:{target}", self.kind())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Status {
    Queued,
    Scheduled,
    Running,
    Paused,
    Failed,
    Done,
    Cancelled,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    id: u64,
    kind: String,
    meeting_id: Option<String>,
    title: Option<String>,
    status: Status,
    error: Option<String>,
    created_at: String,
    /// When the task last began to run; with `finished_at` it times the work.
    started_at: Option<String>,
    finished_at: Option<String>,
    can_retry: bool,
    scheduled_at: Option<String>,
}

struct Entry {
    view: TaskView,
    key: String,
    action: Option<Action>,
    finished: bool,
    retrying: bool,
    scheduled: bool,
}

#[derive(Default)]
struct Queue {
    next_id: u64,
    entries: Vec<Entry>,
}
static QUEUE: LazyLock<Mutex<Queue>> = LazyLock::new(|| Mutex::new(Queue::default()));
static CHANGED: Notify = Notify::const_new();
tokio::task_local! { static CURRENT: u64; }

pub(crate) struct Task(u64);

impl Task {
    pub(crate) fn register(action: Action) -> Self {
        let mut queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
        // A fresh request supersedes a delayed retry of the same action.
        queue
            .entries
            .retain(|entry| !entry.scheduled || entry.key != action.key());
        queue.next_id += 1;
        let id = queue.next_id;
        let view = TaskView {
            id,
            kind: action.kind().into(),
            meeting_id: action.meeting_id().map(str::to_owned),
            title: match &action {
                Action::Import { title, .. } => Some(title.clone()),
                _ => None,
            },
            status: Status::Queued,
            error: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            finished_at: None,
            can_retry: false,
            scheduled_at: None,
        };
        queue.entries.push(Entry {
            key: action.key(),
            action: Some(action),
            view,
            finished: false,
            retrying: false,
            scheduled: false,
        });
        Self(id)
    }

    pub(crate) async fn run<T>(self, operation: impl Future<Output = T>) -> T {
        loop {
            let changed = CHANGED.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let is_first = QUEUE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .entries
                .iter()
                .find(|entry| !entry.finished)
                .is_some_and(|entry| entry.view.id == self.0);
            if is_first {
                break;
            }
            changed.await;
        }
        // Avoid starting preprocessing while capture owns the machine.
        let permit = crate::background_work::acquire().await;
        drop(permit);
        set_status(self.0, Status::Running, None);
        let result = CURRENT.scope(self.0, operation).await;
        finish(self.0, Status::Done);
        result
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        // A panic or a dropped future must not leave the rest of the queue stuck.
        finish(self.0, Status::Failed);
    }
}

fn set_status(id: u64, status: Status, error: Option<String>) {
    if let Some(entry) = QUEUE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entries
        .iter_mut()
        .find(|entry| entry.view.id == id)
    {
        if status == Status::Running {
            entry.view.started_at = Some(chrono::Utc::now().to_rfc3339());
        }
        entry.view.status = status;
        entry.view.error = error;
    }
}

fn finish(id: u64, fallback: Status) {
    let mut queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
    let mut outcome = None;
    if let Some(entry) = queue
        .entries
        .iter_mut()
        .find(|entry| entry.view.id == id && !entry.finished)
    {
        if matches!(entry.view.status, Status::Queued | Status::Running) {
            entry.view.status = fallback;
            if fallback == Status::Failed {
                entry.view.error = Some("The task stopped unexpectedly".into());
            }
        }
        entry.finished = true;
        entry.view.finished_at = Some(chrono::Utc::now().to_rfc3339());
        outcome = entry
            .action
            .clone()
            .map(|action| (action, entry.view.status, entry.view.error.clone()));
        if entry.view.status != Status::Failed {
            entry.action = None;
        }
    }
    // Keep recent history, all pending work and every failure needing attention.
    let completed: Vec<u64> = queue
        .entries
        .iter()
        .filter(|entry| entry.finished && !entry.scheduled && entry.view.status != Status::Failed)
        .map(|entry| entry.view.id)
        .collect();
    let prune = completed.len().saturating_sub(40);
    queue
        .entries
        .retain(|entry| !completed[..prune].contains(&entry.view.id));
    drop(queue);
    if let Some((action, status, error)) = outcome {
        recovery::completed(action, status, error);
    }
    CHANGED.notify_waiters();
}

/// Whether `action` waits in the queue and has not started yet: it will then
/// see every change made before it starts, so a second copy is not needed.
pub(crate) fn is_waiting(action: &Action) -> bool {
    let key = action.key();
    QUEUE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entries
        .iter()
        .any(|entry| !entry.finished && entry.key == key && entry.view.status == Status::Queued)
}

pub(crate) fn fail_current(error: impl ToString) {
    let _ = CURRENT.try_with(|id| set_status(*id, Status::Failed, Some(error.to_string())));
}

pub(crate) fn cancel_current() {
    let _ = CURRENT.try_with(|id| set_status(*id, Status::Cancelled, None));
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tasks: Vec<TaskView>,
    recording_paused: bool,
}

#[tauri::command]
pub fn get_tasks() -> Snapshot {
    let (recording_paused, _) = crate::background_work::recording_status();
    let queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
    let mut entries: Vec<&Entry> = queue.entries.iter().collect();
    entries.sort_by_key(|entry| {
        (
            entry.finished && !entry.scheduled,
            entry.scheduled,
            if entry.view.status == Status::Failed {
                0
            } else {
                1
            },
            entry.view.id,
        )
    });
    let tasks = entries
        .into_iter()
        .map(|entry| {
            let mut view = entry.view.clone();
            if view.status == Status::Running && recording_paused {
                view.status = Status::Paused;
            }
            view.can_retry = entry.finished
                && matches!(view.status, Status::Failed | Status::Scheduled)
                && !entry.retrying
                && !queue
                    .entries
                    .iter()
                    .any(|other| !other.finished && other.key == entry.key);
            view
        })
        .collect();
    Snapshot {
        tasks,
        recording_paused,
    }
}

#[tauri::command]
pub async fn retry_task<R: Runtime>(app: AppHandle<R>, task_id: u64) -> Result<(), String> {
    retry_entry(app, task_id, false).await
}

async fn retry_entry<R: Runtime>(
    app: AppHandle<R>,
    task_id: u64,
    automatic: bool,
) -> Result<(), String> {
    let action = {
        let mut queue = QUEUE.lock().map_err(|_| "The task queue is unavailable")?;
        let entry = queue
            .entries
            .iter()
            .find(|entry| entry.view.id == task_id)
            .ok_or("Task not found")?;
        if !entry.finished
            || !matches!(entry.view.status, Status::Failed | Status::Scheduled)
            || entry.retrying
        {
            return Err("Only failed tasks can be retried".into());
        }
        if queue
            .entries
            .iter()
            .any(|other| !other.finished && other.key == entry.key)
        {
            return Err("This task is already queued".into());
        }
        let entry = queue
            .entries
            .iter_mut()
            .find(|entry| entry.view.id == task_id)
            .unwrap();
        entry.retrying = true;
        entry.action.clone().ok_or("This task cannot be retried")?
    };
    let available = if automatic { recovery::action_available(&app, &action).await } else { Ok(true) };
    let result = match available {
        Ok(false) => {
            recovery::completed(action.clone(), Status::Cancelled, None);
            Ok(())
        }
        Ok(true) => {
            recovery::begin_retry(&action.key(), automatic);
            retry(app, action.clone()).await
        }
        Err(error) => Err(error),
    };
    let mut queue = QUEUE.lock().map_err(|_| "The task queue is unavailable")?;
    if result.is_ok() {
        queue.entries.retain(|entry| entry.view.id != task_id);
    } else if let Some(entry) = queue
        .entries
        .iter_mut()
        .find(|entry| entry.view.id == task_id)
    {
        entry.retrying = false;
        entry.scheduled = false;
        entry.view.status = Status::Failed;
        entry.view.finished_at = Some(chrono::Utc::now().to_rfc3339());
        entry.view.scheduled_at = None;
        entry.view.error = result.as_ref().err().cloned();
    }
    drop(queue);
    if let Err(error) = &result {
        recovery::completed(action, Status::Failed, Some(error.clone()));
    }
    result
}

async fn retry<R: Runtime>(app: AppHandle<R>, action: Action) -> Result<(), String> {
    let pool = app
        .try_state::<crate::state::AppState>()
        .ok_or("The database is unavailable")?
        .db_manager
        .pool()
        .clone();
    if let Some(meeting_id) = action.meeting_id() {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?)")
            .bind(meeting_id)
            .fetch_one(&pool)
            .await
            .map_err(|error| error.to_string())?;
        if !exists {
            recovery::completed(action, Status::Cancelled, None);
            return Ok(());
        }
    }
    match action {
        Action::RecoverSummary { meeting_id } => {
            let data = crate::database::repositories::summary::SummaryProcessesRepository::get_summary_data(&pool, &meeting_id)
                .await.map_err(|error| error.to_string())?;
            if !data.is_some_and(|data| {
                matches!(data.status.to_lowercase().as_str(), "failed" | "error")
            }) {
                recovery::completed(Action::RecoverSummary { meeting_id }, Status::Done, None);
                return Ok(());
            }
            crate::processing::queue_summary(&app, &pool, &meeting_id).await
        }
        Action::Speech { meeting_id } => {
            crate::audio::tts::tts_prepare_summary_audio(app, meeting_id)
        }
        Action::Speakers {
            meeting_id,
            audio_path,
        } => {
            crate::audio::diarization::start_meeting_diarization(app, pool, meeting_id, audio_path)
        }
        Action::Summary {
            meeting_id,
            text,
            provider,
            model,
            prompt,
            template,
            language,
        } => crate::summary::commands::start_summary(
            app,
            pool,
            meeting_id,
            text,
            provider,
            model,
            None,
            None,
            Some(prompt),
            Some(template),
            language,
        )
        .await
        .map(|_| ()),
        Action::Import {
            source,
            title,
            language,
            model,
            provider,
        } => {
            tauri::async_runtime::spawn(async move {
                let _ = crate::audio::import::start_import(
                    app, source, title, language, model, provider,
                )
                .await;
            });
            Ok(())
        }
        Action::Retranscription {
            meeting_id,
            folder,
            language,
            model,
            provider,
            process_after,
        } => {
            tauri::async_runtime::spawn(async move {
                let _ = crate::audio::retranscription::start_with_followup(
                    app,
                    meeting_id,
                    folder,
                    language,
                    model,
                    provider,
                    process_after,
                )
                .await;
            });
            Ok(())
        }
        Action::VoiceMatch { meeting_id } => {
            crate::audio::voiceprint::queue_meeting_matching(app, meeting_id);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn fifo_keeps_one_job_running_and_failure_unblocks_the_next_job() {
        let first = Task::register(Action::Speech {
            meeting_id: "queue-test-first".into(),
        });
        let first_id = first.0;
        let second = Task::register(Action::Speech {
            meeting_id: "queue-test-second".into(),
        });
        let second_id = second.0;
        let active = Arc::new(AtomicUsize::new(0));
        let second_active = active.clone();
        let (release, wait) = tokio::sync::oneshot::channel();
        let (started, observed) = tokio::sync::oneshot::channel();
        // Poll the later task first: acquisition must follow enqueue order.
        let later = tokio::spawn(second.run(async move {
            assert_eq!(second_active.fetch_add(1, Ordering::SeqCst), 0);
            second_active.fetch_sub(1, Ordering::SeqCst);
        }));
        let earlier = tokio::spawn(first.run(async move {
            assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
            started.send(()).unwrap();
            wait.await.unwrap();
            fail_current("fresh failure");
            active.fetch_sub(1, Ordering::SeqCst);
        }));
        timeout(Duration::from_secs(1), observed)
            .await
            .unwrap()
            .unwrap();
        assert!(!later.is_finished());
        release.send(()).unwrap();
        timeout(Duration::from_secs(1), earlier)
            .await
            .unwrap()
            .unwrap();
        timeout(Duration::from_secs(1), later)
            .await
            .unwrap()
            .unwrap();
        let snapshot = get_tasks();
        let failed = snapshot
            .tasks
            .iter()
            .find(|task| task.id == first_id)
            .unwrap();
        assert!(matches!(failed.status, Status::Failed));
        assert_eq!(failed.error.as_deref(), Some("fresh failure"));
        assert!(failed.can_retry);
        assert!(matches!(
            snapshot
                .tasks
                .iter()
                .find(|task| task.id == second_id)
                .unwrap()
                .status,
            Status::Done
        ));
        QUEUE
            .lock()
            .unwrap()
            .entries
            .retain(|entry| entry.view.id != first_id && entry.view.id != second_id);
    }
}
