use serde::{Deserialize, Serialize};

use crate::i18n::{is_russian, tr};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: Option<String>,
    pub title: String,
    pub body: String,
    pub notification_type: NotificationType,
    pub priority: NotificationPriority,
    pub timeout: NotificationTimeout,
    pub icon: Option<String>,
    pub sound: bool,
    pub actions: Vec<NotificationAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationType {
    RecordingStarted,
    RecordingStopped,
    RecordingPaused,
    RecordingResumed,
    TranscriptionComplete,
    MeetingReminder(u64), // Duration in minutes
    SystemError(String),
    Test, // For testing notifications
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationTimeout {
    Never,
    Seconds(u64),
    Default,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    pub id: String,
    pub title: String,
    pub action_type: NotificationActionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationActionType {
    Button,
    Reply,
}

impl Notification {
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        notification_type: NotificationType,
    ) -> Self {
        Self {
            id: None,
            title: title.into(),
            body: body.into(),
            notification_type,
            priority: NotificationPriority::Normal,
            timeout: NotificationTimeout::Default,
            icon: None,
            sound: true,
            actions: vec![],
        }
    }

    pub fn with_priority(mut self, priority: NotificationPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_timeout(mut self, timeout: NotificationTimeout) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn add_action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }
}

impl Default for NotificationPriority {
    fn default() -> Self {
        NotificationPriority::Normal
    }
}

impl Default for NotificationTimeout {
    fn default() -> Self {
        NotificationTimeout::Default
    }
}

/// Body of the "recording started" notification in the interface language.
pub fn recording_started_body(meeting_name: Option<String>) -> String {
    match meeting_name {
        Some(name) if is_russian() => format!("Запись встречи «{}» начата", name),
        Some(name) => format!("Recording started for meeting: {}", name),
        None => tr(
            "Recording has started. Please inform others in the meeting that you are recording.",
            "Запись начата. Предупредите участников встречи, что идёт запись.",
        )
        .to_string(),
    }
}

// Helper functions for creating common notifications
impl Notification {
    pub fn recording_started(meeting_name: Option<String>) -> Self {
        let body = recording_started_body(meeting_name);

        Notification::new("Meetily", body, NotificationType::RecordingStarted)
            .with_priority(NotificationPriority::High)
            .with_timeout(NotificationTimeout::Seconds(5))
    }

    pub fn recording_stopped() -> Self {
        Notification::new(
            "Meetily",
            tr("Recording has been stopped and saved", "Запись остановлена и сохранена"),
            NotificationType::RecordingStopped,
        )
        .with_priority(NotificationPriority::Normal)
        .with_timeout(NotificationTimeout::Seconds(3))
    }

    pub fn recording_paused() -> Self {
        Notification::new(
            "Meetily",
            tr("Recording has been paused", "Запись приостановлена"),
            NotificationType::RecordingPaused,
        )
        .with_priority(NotificationPriority::Normal)
        .with_timeout(NotificationTimeout::Seconds(3))
    }

    pub fn recording_resumed() -> Self {
        Notification::new(
            "Meetily",
            tr("Recording has been resumed", "Запись возобновлена"),
            NotificationType::RecordingResumed,
        )
        .with_priority(NotificationPriority::Normal)
        .with_timeout(NotificationTimeout::Seconds(3))
    }

    pub fn transcription_complete(file_path: Option<String>) -> Self {
        let body = match file_path {
            Some(path) if is_russian() => format!("Транскрипция завершена и сохранена: {}", path),
            Some(path) => format!("Transcription completed and saved to: {}", path),
            None => tr("Transcription has been completed", "Транскрипция завершена").to_string(),
        };

        Notification::new("Meetily", body, NotificationType::TranscriptionComplete)
            .with_priority(NotificationPriority::Normal)
            .with_timeout(NotificationTimeout::Seconds(5))
    }

    pub fn meeting_reminder(minutes_until: u64, meeting_title: Option<String>) -> Self {
        let body = match meeting_title {
            Some(title) if is_russian() => {
                format!("Встреча «{}» начнётся через {} мин", title, minutes_until)
            }
            Some(title) => format!("Meeting '{}' starts in {} minutes", title, minutes_until),
            None if is_russian() => format!("Встреча начнётся через {} мин", minutes_until),
            None => format!("Meeting starts in {} minutes", minutes_until),
        };

        Notification::new(
            "Meetily",
            body,
            NotificationType::MeetingReminder(minutes_until),
        )
        .with_priority(NotificationPriority::High)
        .with_timeout(NotificationTimeout::Seconds(10))
    }

    pub fn system_error(error: impl Into<String>) -> Self {
        let error_string = error.into();
        Notification::new(
            tr("Meetily Error", "Ошибка Meetily"),
            error_string.clone(),
            NotificationType::SystemError(error_string),
        )
        .with_priority(NotificationPriority::Critical)
        .with_timeout(NotificationTimeout::Never)
    }

    pub fn test_notification() -> Self {
        Notification::new(
            "Meetily",
            tr(
                "This is a test notification to verify the system is working correctly",
                "Это тестовое уведомление для проверки работы системы",
            ),
            NotificationType::Test,
        )
        .with_priority(NotificationPriority::Normal)
        .with_timeout(NotificationTimeout::Seconds(5))
    }
}
