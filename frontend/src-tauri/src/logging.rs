//! Application log: every line goes to stderr (the dev console) and, once the
//! app data folder is known, to a daily file `logs/ru_meet.YYYY-MM-DD.log` in
//! it, so a release build without a console still keeps its log. Launches on
//! the same day append to that day's file; the last `KEEP_FILES` days are
//! kept. A day's file stops growing at `MAX_DAY_BYTES`, so a runaway log cannot
//! fill the disk. Lines logged before the folder is known are kept in memory
//! and written first. The background worker logs to its stderr, which the app
//! forwards here (`background_process`).

use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Runtime};

const FILE_PREFIX: &str = "ru_meet.";
const FILE_SUFFIX: &str = ".log";
/// Daily files kept, today's included: a week.
const KEEP_FILES: usize = 7;
const MAX_DAY_BYTES: u64 = 50 * 1024 * 1024;
/// Startup lines kept until the log file is opened.
const EARLY_LIMIT: usize = 256 * 1024;
/// Default amount of the log's end returned for viewing.
const TAIL_BYTES: u64 = 512 * 1024;

struct Sink {
    dir: Option<PathBuf>,
    file: Option<File>,
    path: Option<PathBuf>,
    /// Date of the open file, `YYYY-MM-DD`.
    day: String,
    /// Size of the open file.
    size: u64,
    early: Vec<u8>,
}

static SINK: Mutex<Sink> = Mutex::new(Sink {
    dir: None,
    file: None,
    path: None,
    day: String::new(),
    size: 0,
    early: Vec::new(),
});

fn sink() -> std::sync::MutexGuard<'static, Sink> {
    SINK.lock().unwrap_or_else(|error| error.into_inner())
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn day_file(dir: &Path, day: &str) -> PathBuf {
    dir.join(format!("{FILE_PREFIX}{day}{FILE_SUFFIX}"))
}

/// Daily log files in `dir`, oldest first (the date sorts as text).
fn day_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX))
                .is_some_and(|day| {
                    day.len() == 10 && chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok()
                })
        })
        .collect();
    files.sort();
    files
}

fn remove_old_files(dir: &Path) {
    let files = day_files(dir);
    let excess = files.len().saturating_sub(KEEP_FILES);
    for path in &files[..excess] {
        let _ = std::fs::remove_file(path);
    }
}

/// Open (append to) the file of `day`, dropping files beyond the last week.
fn open_day(sink: &mut Sink, dir: &Path, day: String) -> io::Result<()> {
    let path = day_file(dir, &day);
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    sink.size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    sink.file = Some(file);
    sink.path = Some(path);
    sink.day = day;
    remove_old_files(dir);
    Ok(())
}

fn write_capped(sink: &mut Sink, bytes: &[u8]) {
    let Some(file) = sink.file.as_mut() else { return };
    if sink.size >= MAX_DAY_BYTES {
        return;
    }
    if sink.size + bytes.len() as u64 > MAX_DAY_BYTES {
        let notice = b"--- The log reached its daily size limit; further lines today are not saved ---\n";
        let _ = file.write_all(notice);
        sink.size = MAX_DAY_BYTES;
        return;
    }
    if file.write_all(bytes).is_ok() {
        sink.size += bytes.len() as u64;
    }
}

fn append(bytes: &[u8]) {
    let mut sink = sink();
    let Some(dir) = sink.dir.clone() else {
        if sink.early.len() < EARLY_LIMIT {
            sink.early.extend_from_slice(bytes);
        }
        return;
    };
    let day = today();
    if sink.day != day {
        sink.file = None;
        if open_day(&mut sink, &dir, day).is_err() {
            return;
        }
    }
    write_capped(&mut sink, bytes);
}

/// Writes each log line to stderr and the log file.
struct Tee;

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stderr().write_all(buf);
        append(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        if let Some(file) = sink().file.as_mut() {
            file.flush()?;
        }
        Ok(())
    }
}

/// Install the logger; `RUST_LOG` selects the levels as before.
pub fn init() {
    env_logger::Builder::from_env(env_logger::Env::default())
        .target(env_logger::Target::Pipe(Box::new(Tee)))
        .init();
}

/// A line the background worker wrote to its stderr.
pub(crate) fn forward_line(line: &str) {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    let _ = Tee.write(&bytes);
}

fn logs_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|dir| dir.join("logs"))
        .map_err(|error| format!("Could not resolve the app data folder: {error}"))
}

/// Start writing the log file; called once the app is set up.
pub fn attach<R: Runtime>(app: &AppHandle<R>) {
    let opened = logs_dir(app).and_then(|dir| {
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let mut sink = sink();
        open_day(&mut sink, &dir, today()).map_err(|error| error.to_string())?;
        sink.dir = Some(dir);
        // Launches of the same day share a file; mark where this one starts.
        let start = format!(
            "\n=== Ru-Meet {} started {} ===\n",
            app.package_info().version,
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        write_capped(&mut sink, start.as_bytes());
        let early = std::mem::take(&mut sink.early);
        write_capped(&mut sink, &early);
        Ok(sink.path.clone())
    });
    match opened {
        Ok(Some(path)) => log::info!("Writing the log to {}", path.display()),
        Ok(None) => {}
        Err(error) => log::warn!("The log is not written to a file: {error}"),
    }
}

#[derive(Serialize)]
pub struct LogTail {
    path: String,
    text: String,
    /// The log is longer than `text`; its start was left out.
    truncated: bool,
}

/// The end of the current log file, for viewing in the app.
#[tauri::command]
pub fn get_log_tail(max_bytes: Option<u64>) -> Result<LogTail, String> {
    let path = {
        let mut sink = sink();
        if let Some(file) = sink.file.as_mut() {
            let _ = file.flush();
        }
        sink.path.clone().ok_or("The log is not written to a file")?
    };
    let mut file = File::open(&path).map_err(|error| format!("Could not open the log: {error}"))?;
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let limit = max_bytes.unwrap_or(TAIL_BYTES).max(1);
    let start = length.saturating_sub(limit);
    file.seek(SeekFrom::Start(start)).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        // Drop the line cut in the middle.
        if let Some(newline) = text.find('\n') {
            text.drain(..=newline);
        }
    }
    Ok(LogTail {
        path: path.display().to_string(),
        text,
        truncated: start > 0,
    })
}

/// Open the folder with the daily logs.
#[tauri::command]
pub fn open_logs_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = logs_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open the logs folder: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_week_of_daily_files() {
        let dir = std::env::temp_dir().join(format!("ru-meet-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for day in 1..=9 {
            std::fs::write(day_file(&dir, &format!("2026-11-{day:02}")), b"x").unwrap();
        }
        std::fs::write(dir.join("ru_meet.notes.log"), b"other").unwrap();

        remove_old_files(&dir);

        let names: Vec<String> = day_files(&dir)
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.first().map(String::as_str), Some("ru_meet.2026-11-03.log"));
        assert_eq!(names.len(), KEEP_FILES);
        assert!(dir.join("ru_meet.notes.log").exists(), "unrelated files stay");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
