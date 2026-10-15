//! Which processes capturing the microphone count as a call (ADR 0004).
//!
//! Attribution is by allowlist: "some app opened the microphone" also fires
//! for dictation, a browser checking levels, Krisp or NVIDIA Broadcast holding
//! the physical microphone all day. A missed meeting costs one click; a false
//! one records whatever was said in the room.
use serde::Serialize;

/// How a process is matched against an app list entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Identifier {
    /// macOS bundle id, matched by prefix: `com.google.Chrome` covers its
    /// `.helper` processes, `com.microsoft.teams` covers `com.microsoft.teams2`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    BundleId(String),
    /// Windows executable name, matched exactly.
    Executable(String),
}

/// A process currently capturing from an input device, never Ru-Meet itself.
#[derive(Debug, Clone)]
pub(crate) struct AudioClient {
    /// What settings show and what a user would type into a list.
    pub display: String,
    /// The process itself first, then (Windows) its ancestors: new Teams
    /// captures through `msedgewebview2.exe`, Chrome through a child process.
    pub identifiers: Vec<Identifier>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicHolder {
    pub name: String,
    /// Whether this process counts as a call.
    pub call: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MicHolders {
    pub holders: Vec<MicHolder>,
}

impl MicHolders {
    pub(crate) fn call_active(&self) -> bool {
        self.holders.iter().any(|holder| holder.call)
    }
}

#[cfg(target_os = "macos")]
pub(crate) const DEFAULT_CALL_APPS: &[&str] = &[
    // Meeting apps.
    "us.zoom.xos",
    "com.microsoft.teams",
    "com.microsoft.SkypeForBusiness",
    "com.skype.skype",
    "com.cisco.webexmeetingsapp",
    "com.webex.meetingmanager",
    "Cisco-Systems.Spark",
    "com.tinyspeck.slackmacgap",
    "ru.yandex.desktop.telemost",
    "ru.unlimitedtech.express.desktop",
    "kontur.talk",
    "vc.dion.desktop",
    // Messengers with calls.
    "ru.keepcoder.Telegram",
    "com.tdesktop.Telegram",
    "net.whatsapp.WhatsApp",
    "org.whispersystems.signal-desktop",
    "com.viber.osx",
    "jp.naver.line.mac",
    "com.tencent.xinWeChat",
    "com.apple.FaceTime",
    "com.hnc.Discord",
    // Browsers: Meet, Telemost, web Zoom and installed PWAs run in a tab.
    "com.google.Chrome",
    "org.mozilla.firefox",
    "com.apple.Safari",
    "com.apple.WebKit.GPU",
    "company.thebrowser.Browser",
    "company.thebrowser.dia",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "com.operasoftware.Opera",
    "com.vivaldi.Vivaldi",
    "org.chromium.Chromium",
    "ru.yandex.desktop.yandex-browser",
    "com.duckduckgo.macos.browser",
    "com.kagi.kagimacOS",
    "app.zen-browser.zen",
    "ai.perplexity.comet",
];

#[cfg(not(target_os = "macos"))]
pub(crate) const DEFAULT_CALL_APPS: &[&str] = &[
    // Meeting apps.
    "zoom.exe",
    "ms-teams.exe",
    "teams.exe",
    "lync.exe",
    "skype.exe",
    "webex.exe",
    "ciscocollabhost.exe",
    "atmgr.exe",
    "slack.exe",
    // Messengers with calls.
    "telegram.exe",
    "whatsapp.exe",
    "whatsapp.root.exe",
    "signal.exe",
    "viber.exe",
    "discord.exe",
    // Browsers: Meet, Telemost, web Zoom and installed PWAs run in a tab.
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "browser.exe",
    "opera.exe",
    "brave.exe",
    "vivaldi.exe",
    "arc.exe",
];

/// Never a call, whatever the lists say: dictation holds the microphone for
/// exactly as long as you speak, and audio utilities hold it all the time.
#[cfg(target_os = "macos")]
const ALWAYS_IGNORED: &[&str] = &[
    "com.apple.VoiceMemos",
    "com.apple.assistantd",
    "com.apple.Siri",
    "com.apple.speech.SpeechRecognitionCore.speechrecognitiond",
    "com.apple.SpeechRecognitionCore.speechrecognitiond",
    "com.prakashjoshipax.VoiceInk",
    "ai.krisp.krispMac",
];

#[cfg(not(target_os = "macos"))]
const ALWAYS_IGNORED: &[&str] = &[
    "audiodg.exe",
    "krisp.exe",
    "nvidia broadcast.exe",
    "voicemeeter.exe",
    "voicemeeterpro.exe",
    "voicemeeter8.exe",
    "voicemeeter8x64.exe",
];

fn identifier_matches(identifier: &Identifier, entry: &str) -> bool {
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    match identifier {
        Identifier::BundleId(id) => id.to_lowercase().starts_with(&entry.to_lowercase()),
        Identifier::Executable(name) => name.eq_ignore_ascii_case(entry),
    }
}

fn belongs<'a>(client: &AudioClient, entries: impl IntoIterator<Item = &'a str> + Clone) -> bool {
    client.identifiers.iter().any(|identifier| {
        entries
            .clone()
            .into_iter()
            .any(|entry| identifier_matches(identifier, entry))
    })
}

/// The judgement itself, over a given list of clients, so it can be tested
/// without asking the system.
pub(crate) fn evaluate(
    clients: &[AudioClient],
    call_apps: &[String],
    excluded_apps: &[String],
    any_app: bool,
) -> MicHolders {
    let mut holders: Vec<MicHolder> = Vec::new();
    for client in clients {
        let ignored = belongs(client, ALWAYS_IGNORED.iter().copied())
            || belongs(client, excluded_apps.iter().map(String::as_str));
        let listed = belongs(client, DEFAULT_CALL_APPS.iter().copied())
            || belongs(client, call_apps.iter().map(String::as_str));
        let call = !ignored && (any_app || listed);
        match holders.iter_mut().find(|holder| holder.name == client.display) {
            Some(holder) => holder.call |= call,
            None => holders.push(MicHolder {
                name: client.display.clone(),
                call,
            }),
        }
    }
    MicHolders { holders }
}

/// Normalise a user-entered app list: Windows takes `.exe` names, macOS
/// bundle ids. Lower-cased, since matching ignores case anyway.
pub(crate) fn normalize_app_list(apps: Vec<String>) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for name in apps {
        let name = name.trim().to_lowercase();
        if !valid_entry(&name) {
            return Err(entry_hint().into());
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

#[cfg(target_os = "macos")]
fn valid_entry(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

#[cfg(target_os = "macos")]
fn entry_hint() -> &'static str {
    "Enter an application bundle id, such as com.spotify.client"
}

#[cfg(not(target_os = "macos"))]
fn valid_entry(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name.ends_with(".exe")
}

#[cfg(not(target_os = "macos"))]
fn entry_hint() -> &'static str {
    "Enter an application filename, such as Spotify.exe"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(names: &[&str]) -> AudioClient {
        AudioClient {
            display: names[0].into(),
            identifiers: names
                .iter()
                .map(|name| Identifier::Executable((*name).into()))
                .collect(),
        }
    }

    fn bundle(id: &str) -> AudioClient {
        AudioClient {
            display: id.into(),
            identifiers: vec![Identifier::BundleId(id.into())],
        }
    }

    #[test]
    fn bundle_ids_match_by_prefix_and_executables_exactly() {
        let teams2 = Identifier::BundleId("com.microsoft.teams2".into());
        assert!(identifier_matches(&teams2, "com.microsoft.teams"));
        assert!(identifier_matches(&teams2, "COM.Microsoft.Teams"));
        assert!(!identifier_matches(&teams2, "com.microsoft.edgemac"));
        let chrome = Identifier::Executable("Chrome.exe".into());
        assert!(identifier_matches(&chrome, "chrome.exe"));
        assert!(!identifier_matches(&chrome, "chrome"));
        assert!(!identifier_matches(&chrome, ""));
    }

    #[test]
    fn only_listed_apps_count_unless_any_app_is_on() {
        let clients = [exe(&["zoom.exe"]), exe(&["photobooth.exe"])];
        let holders = evaluate(&clients, &[], &[], false);
        assert_eq!(
            holders.holders,
            vec![
                MicHolder { name: "zoom.exe".into(), call: true },
                MicHolder { name: "photobooth.exe".into(), call: false },
            ]
        );
        assert!(evaluate(&clients[1..], &[], &[], true).call_active());
        assert!(evaluate(&clients[1..], &["photobooth.exe".into()], &[], false).call_active());
    }

    #[test]
    fn a_helper_counts_through_its_ancestor() {
        let webview = exe(&["msedgewebview2.exe", "msedgewebview2.exe", "ms-teams.exe"]);
        assert!(evaluate(&[webview.clone()], &[], &[], false).call_active());
        assert!(!evaluate(&[exe(&["msedgewebview2.exe", "explorer.exe"])], &[], &[], false)
            .call_active());
        // Excluding the app excludes its helpers too.
        assert!(!evaluate(&[webview], &[], &["ms-teams.exe".into()], false).call_active());
    }

    #[test]
    fn dictation_and_audio_utilities_never_count() {
        let client = |name: &str| if cfg!(target_os = "macos") { bundle(name) } else { exe(&[name]) };
        let clients: Vec<_> = ALWAYS_IGNORED.iter().map(|name| client(name)).collect();
        let listed: Vec<String> = ALWAYS_IGNORED.iter().map(|name| name.to_string()).collect();
        assert!(!evaluate(&clients, &listed, &[], true).call_active());
        assert!(!evaluate(&[exe(&["zoom.exe"])], &[], &["zoom.exe".into()], true).call_active());
    }

    #[test]
    fn processes_of_one_app_are_listed_once() {
        let clients = [exe(&["chrome.exe"]), exe(&["chrome.exe"])];
        assert_eq!(evaluate(&clients, &[], &[], false).holders.len(), 1);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn windows_lists_take_exe_names_without_paths() {
        assert_eq!(
            normalize_app_list(vec![" Spotify.exe ".into(), "spotify.EXE".into()]).unwrap(),
            vec!["spotify.exe"]
        );
        assert!(normalize_app_list(vec![r"C:\Apps\Spotify.exe".into()]).is_err());
        assert!(normalize_app_list(vec!["spotify".into()]).is_err());
        assert!(normalize_app_list(vec!["".into()]).is_err());
        assert!(normalize_app_list(Vec::new()).unwrap().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_lists_take_bundle_ids() {
        assert_eq!(
            normalize_app_list(vec![" com.Spotify.client ".into()]).unwrap(),
            vec!["com.spotify.client"]
        );
        assert!(normalize_app_list(vec!["/Applications/Spotify.app".into()]).is_err());
        assert!(normalize_app_list(vec!["".into()]).is_err());
    }
}
