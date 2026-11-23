//! Default titles of new recordings, readable in the meeting list until the
//! user renames the meeting or a summary names it: "Встреча 23 ноября, 10:53",
//! or "Zoom · 23 ноября, 10:53" for a detected call. The webview builds the
//! same title for recordings it starts (`frontend/src/lib/meetingTitle.ts`).

use chrono::{DateTime, Datelike, Local, Timelike};

const MONTHS_RU: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября",
    "октября", "ноября", "декабря",
];
const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn format_title(app: Option<&str>, at: DateTime<Local>, russian: bool) -> String {
    let month = at.month0() as usize;
    let when = if russian {
        format!("{} {}, {:02}:{:02}", at.day(), MONTHS_RU[month], at.hour(), at.minute())
    } else {
        format!("{} {}, {:02}:{:02}", MONTHS_EN[month], at.day(), at.hour(), at.minute())
    };
    match app {
        Some(app) => format!("{app} · {when}"),
        None if russian => format!("Встреча {when}"),
        None => format!("Meeting {when}"),
    }
}

/// Title of a recording starting now; `app` names the detected call app.
pub fn default_meeting_title(app: Option<&str>) -> String {
    format_title(app, Local::now(), crate::i18n::is_russian())
}

/// Readable call app name from its process name: "Zoom.exe" -> "Zoom".
pub fn app_display_name(process: &str) -> Option<String> {
    let name = process.trim();
    let name = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name)
        .trim();
    let mut chars = name.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn titles_read_as_date_and_time() {
        let at = Local.with_ymd_and_hms(2026, 11, 23, 9, 5, 41).unwrap();
        assert_eq!(format_title(None, at, true), "Встреча 23 ноября, 09:05");
        assert_eq!(format_title(None, at, false), "Meeting Nov 23, 09:05");
        assert_eq!(format_title(Some("Zoom"), at, true), "Zoom · 23 ноября, 09:05");
    }

    #[test]
    fn app_names_drop_the_extension() {
        assert_eq!(app_display_name("Zoom.exe").as_deref(), Some("Zoom"));
        assert_eq!(app_display_name("telegram").as_deref(), Some("Telegram"));
        assert_eq!(app_display_name("  "), None);
    }
}
