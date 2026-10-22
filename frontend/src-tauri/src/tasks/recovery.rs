//! Failed jobs are durable; startup retries are dormant queue entries until the
//! grace period ends. No model or audio file is opened to discover these jobs.
use super::{Action, Entry, Queue, Status, TaskView, QUEUE};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager, Runtime};

const STARTUP_GRACE: Duration = Duration::from_secs(10 * 60);
const MAX_AUTOMATIC_ATTEMPTS: u8 = 3;

#[derive(Clone, Serialize, Deserialize)]
struct Failure {
    action: Action,
    error: String,
    failed_at: String,
    #[serde(default)]
    automatic_attempts: u8,
}

#[derive(Default)]
struct Journal {
    path: Option<PathBuf>,
    failures: BTreeMap<String, Failure>,
}

impl Journal {
    fn open(path: PathBuf) -> Result<Self, String> {
        let failures = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(Self {
            path: Some(path),
            failures,
        })
    }

    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let directory = path.parent().ok_or("Invalid failure journal path")?;
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let mut file =
            tempfile::NamedTempFile::new_in(directory).map_err(|error| error.to_string())?;
        serde_json::to_writer(file.as_file_mut(), &self.failures)
            .map_err(|error| error.to_string())?;
        file.persist(path).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn completed(&mut self, action: Action, status: Status, error: Option<String>) {
        let key = action.key();
        if status == Status::Failed {
            let automatic_attempts = self
                .failures
                .get(&key)
                .map_or(0, |failure| failure.automatic_attempts);
            self.failures.insert(
                key,
                Failure {
                    action,
                    error: error.unwrap_or_else(|| "The task stopped unexpectedly".into()),
                    failed_at: chrono::Utc::now().to_rfc3339(),
                    automatic_attempts,
                },
            );
        } else if self.failures.remove(&key).is_none() {
            return;
        }
        if let Err(error) = self.save() {
            log::warn!("Could not save failed tasks: {error}");
        }
    }

    fn begin_retry(&mut self, key: &str, automatic: bool) {
        if let Some(failure) = self.failures.get_mut(key) {
            failure.automatic_attempts = if automatic {
                failure.automatic_attempts.saturating_add(1)
            } else {
                0
            };
            if let Err(error) = self.save() {
                log::warn!("Could not save retry attempt: {error}");
            }
        }
    }
}

static JOURNAL: LazyLock<Mutex<Journal>> = LazyLock::new(|| Mutex::new(Journal::default()));

pub(super) fn completed(action: Action, status: Status, error: Option<String>) {
    JOURNAL
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .completed(action, status, error);
}

pub(super) fn begin_retry(key: &str, automatic: bool) {
    JOURNAL
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .begin_retry(key, automatic);
}

/// Unavailable optional summary models are skipped, never held for install.
pub(super) async fn action_available<R: Runtime>(app: &AppHandle<R>, action: &Action) -> Result<bool, String> {
    match action {
        Action::Summary { provider, model, .. } => Ok(crate::summary::commands::summary_model_available(app, provider, model)),
        Action::RecoverSummary { .. } => {
            let state = app.try_state::<crate::state::AppState>().ok_or("The database is unavailable")?;
            crate::processing::automatic_summary_available(app, state.db_manager.pool()).await
        }
        _ => Ok(true),
    }
}

impl Queue {
    fn restore_failure(&mut self, failure: Failure, due: &str) {
        let key = failure.action.key();
        // A task requested in this session always supersedes saved history.
        if self.entries.iter().any(|entry| entry.key == key) {
            return;
        }
        let scheduled = failure.automatic_attempts < MAX_AUTOMATIC_ATTEMPTS;
        self.next_id += 1;
        self.entries.push(Entry {
            key,
            view: TaskView {
                id: self.next_id,
                kind: failure.action.kind().into(),
                meeting_id: failure.action.meeting_id().map(str::to_owned),
                title: match &failure.action {
                    Action::Import { title, .. } => Some(title.clone()),
                    _ => None,
                },
                status: if scheduled {
                    Status::Scheduled
                } else {
                    Status::Failed
                },
                error: Some(failure.error),
                created_at: failure.failed_at.clone(),
                finished_at: if scheduled {
                    None
                } else {
                    Some(failure.failed_at)
                },
                can_retry: true,
                scheduled_at: scheduled.then(|| due.to_string()),
            },
            action: Some(failure.action),
            finished: true,
            retrying: false,
            scheduled,
        });
    }

    fn next_recovery(&self, recording: bool, not_before: tokio::time::Instant) -> Option<u64> {
        if tokio::time::Instant::now() < not_before
            || recording
            || self
                .entries
                .iter()
                .any(|entry| !entry.finished || entry.retrying)
        {
            return None;
        }
        self.entries
            .iter()
            .find(|entry| entry.scheduled)
            .map(|entry| entry.view.id)
    }
}

/// Existing versions persist summary errors in SQLite, but no other job errors.
/// Import only explicit failures, never merely old or incomplete recordings.
async fn saved_summary_failures(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<(String, Option<String>)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT s.meeting_id, s.error FROM summary_processes s JOIN meetings m ON m.id = s.meeting_id \
         WHERE LOWER(s.status) IN ('failed', 'error') \
         ORDER BY s.updated_at DESC LIMIT 50"
    ).fetch_all(pool).await
}

pub(crate) fn initialize<R: Runtime>(app: AppHandle<R>) {
    let deadline = tokio::time::Instant::now() + STARTUP_GRACE;
    let due = (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    tauri::async_runtime::spawn(async move {
        // Leave initial window/device setup alone. Only a small journal and a
        // bounded metadata query are needed; models load after the deadline.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let pool = loop {
            if let Some(state) = app.try_state::<crate::state::AppState>() {
                break state.db_manager.pool().clone();
            }
            // The first-launch/database recovery screen may create it later.
            tokio::time::sleep(Duration::from_secs(5)).await;
        };
        let path = match crate::portable::app_data_dir(&app) {
            Ok(path) => path.join("failed-tasks.json"),
            Err(error) => {
                log::warn!("Could not resolve failed tasks: {error}");
                return;
            }
        };
        let disk = match tokio::task::spawn_blocking(move || Journal::open(path)).await {
            Ok(Ok(journal)) => journal,
            result => {
                log::warn!(
                    "Could not restore failed tasks: {}",
                    match result {
                        Ok(Err(error)) => error,
                        Err(error) => error.to_string(),
                        _ => unreachable!(),
                    }
                );
                return;
            }
        };
        let existing = saved_summary_failures(&pool).await.unwrap_or_else(|error| {
            log::warn!("Could not read saved summary errors: {error}");
            Vec::new()
        });
        let session_keys: std::collections::HashSet<String> = QUEUE
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        let failures = {
            let mut journal = JOURNAL.lock().unwrap_or_else(|error| error.into_inner());
            journal.path = disk.path;
            // Preserve any failure that occurred while metadata was loading.
            for (key, failure) in disk.failures {
                if !session_keys.contains(&key) {
                    journal.failures.entry(key).or_insert(failure);
                }
            }
            for (meeting_id, error) in existing {
                let action = Action::RecoverSummary { meeting_id };
                if session_keys.contains(&action.key()) {
                    continue;
                }
                journal.failures.entry(action.key()).or_insert(Failure {
                    action,
                    error: error.unwrap_or_else(|| "Summary generation failed".into()),
                    failed_at: chrono::Utc::now().to_rfc3339(),
                    automatic_attempts: 0,
                });
            }
            if let Err(error) = journal.save() {
                log::warn!("Could not save restored failures: {error}");
            }
            journal.failures.values().cloned().collect::<Vec<_>>()
        };
        let mut available_failures = Vec::new();
        for failure in failures {
            if action_available(&app, &failure.action).await.unwrap_or(true) {
                available_failures.push(failure);
            } else {
                completed(failure.action, Status::Cancelled, None);
            }
        }
        {
            let mut queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
            log::info!(
                "Restoring {} saved failed tasks; automatic retries wait until {due}",
                available_failures.len()
            );
            for failure in available_failures {
                queue.restore_failure(failure, &due);
            }
        }
        tokio::time::sleep_until(deadline).await;
        loop {
            let next = {
                let (recording, _) = crate::background_work::recording_status();
                let queue = QUEUE.lock().unwrap_or_else(|error| error.into_inner());
                if !queue.entries.iter().any(|entry| entry.scheduled) {
                    return;
                }
                queue.next_recovery(recording, deadline)
            };
            if let Some(id) = next {
                log::info!("Starting deferred retry of task {id}");
                if let Err(error) = super::retry_entry(app.clone(), id, true).await {
                    log::warn!("Deferred task {id} could not start: {error}");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn failure() -> Failure {
        Failure {
            action: Action::Speech {
                meeting_id: "saved-meeting".into(),
            },
            error: "old error".into(),
            failed_at: "2026-10-21T10:00:00Z".into(),
            automatic_attempts: 0,
        }
    }

    #[test]
    fn failures_and_retry_limit_survive_restart_success_and_cancel_clear_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("failed-tasks.json");
        let action = failure().action;
        let mut journal = Journal::open(path.clone()).unwrap();
        journal.completed(action.clone(), Status::Failed, Some("failed".into()));
        for _ in 0..3 {
            journal = Journal::open(path.clone()).unwrap();
            journal.begin_retry(&action.key(), true);
            journal.completed(action.clone(), Status::Failed, Some("still failed".into()));
        }
        journal = Journal::open(path.clone()).unwrap();
        assert_eq!(journal.failures[&action.key()].automatic_attempts, 3);
        let mut queue = Queue::default();
        queue.restore_failure(journal.failures[&action.key()].clone(), "later");
        assert!(queue
            .next_recovery(false, tokio::time::Instant::now())
            .is_none());
        assert!(queue.entries[0].view.status == Status::Failed);
        journal.begin_retry(&action.key(), false);
        assert_eq!(
            Journal::open(path.clone()).unwrap().failures[&action.key()].automatic_attempts,
            0
        );
        journal.completed(action.clone(), Status::Done, None);
        assert!(Journal::open(path.clone()).unwrap().failures.is_empty());
        journal.completed(action.clone(), Status::Failed, None);
        journal.completed(action, Status::Cancelled, None);
        assert!(Journal::open(path).unwrap().failures.is_empty());
    }

    #[test]
    fn dormant_retries_do_not_hold_fifo_and_wait_for_recording_and_fresh_work() {
        let mut queue = Queue::default();
        queue.restore_failure(failure(), "later");
        queue.restore_failure(failure(), "later");
        assert_eq!(queue.entries.len(), 1);
        assert!(queue
            .next_recovery(false, tokio::time::Instant::now() + STARTUP_GRACE)
            .is_none());
        assert!(!queue.entries.iter().any(|entry| !entry.finished));
        assert!(queue
            .next_recovery(true, tokio::time::Instant::now())
            .is_none());
        assert_eq!(
            queue.next_recovery(false, tokio::time::Instant::now()),
            Some(1)
        );
        let mut current = failure();
        current.action = Action::Speech {
            meeting_id: "fresh-meeting".into(),
        };
        queue.restore_failure(current, "later");
        queue.entries[1].scheduled = false;
        queue.entries[1].finished = false;
        assert!(queue
            .next_recovery(false, tokio::time::Instant::now())
            .is_none());
        queue.entries[1].finished = true;
        assert_eq!(
            queue.next_recovery(false, tokio::time::Instant::now()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn recovery_reads_only_explicit_errors_of_existing_meetings() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE meetings(id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE summary_processes(meeting_id TEXT, status TEXT, error TEXT, updated_at TEXT)").execute(&pool).await.unwrap();
        for (id, status) in [
            ("failed", "FAILED"),
            ("error", "error"),
            ("done", "completed"),
            ("pending", "pending"),
            ("cancelled", "cancelled"),
            ("deleted", "failed"),
        ] {
            if id != "deleted" {
                sqlx::query("INSERT INTO meetings VALUES(?)")
                    .bind(id)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query("INSERT INTO summary_processes VALUES(?, ?, 'test error', '2026-10-21')")
                .bind(id)
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
        }
        let mut rows = saved_summary_failures(&pool).await.unwrap();
        rows.sort();
        assert_eq!(
            rows,
            vec![
                ("error".into(), Some("test error".into())),
                ("failed".into(), Some("test error".into()))
            ]
        );
    }
}
