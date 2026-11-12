//! "Looks like a meeting started — record it?" reminders.
//!
//! Windows keeps a per-app record of microphone use (the same data behind the
//! mic icon in the taskbar). A meeting app whose last microphone session has
//! started but not stopped is in a call right now. Bolo only reads these
//! timestamps; it never listens to anything until the user clicks Start.
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
pub struct Detected { pub app: String }

/// Apps that are normally only on the microphone during a call or meeting.
/// Browsers count because Google Meet, Teams-web and Zoom-web run in them.
const MEETING_APPS: &[(&str, &str)] = &[
    ("zoom", "Zoom"), ("ms-teams", "Microsoft Teams"), ("msteams", "Microsoft Teams"), ("teams", "Microsoft Teams"),
    ("webex", "Webex"), ("ciscocollabhost", "Webex"), ("slack", "Slack"), ("whatsapp", "WhatsApp"),
    ("skype", "Skype"), ("discord", "Discord"), ("googlemeet", "Google Meet"),
    ("chrome", "a browser"), ("msedge", "a browser"), ("brave", "a browser"), ("firefox", "a browser"), ("opera", "a browser"),
];

/// Maps a registry key name (an exe path with `#` separators, or a package
/// family name) to a friendly app name, if it's a meeting app.
pub fn meeting_app(key: &str) -> Option<&'static str> {
    let last = key.rsplit(['#', '\\']).next().unwrap_or(key).to_lowercase();
    let stem = last.trim_end_matches(".exe");
    let stem = stem.split('_').next().unwrap_or(stem); // package family: Name_publisherid
    let stem = stem.rsplit('.').next().unwrap_or(stem); // 5319275A.WhatsAppDesktop → whatsappdesktop
    if stem.contains("bolo") { return None; }
    MEETING_APPS.iter().find(|(needle, _)| stem.starts_with(needle)).map(|(_, name)| *name)
}

#[cfg(windows)]
fn apps_using_microphone() -> Vec<&'static str> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let mut found = vec![];
    let Ok(base) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone")
    else { return found; };
    let mut check = |key: &RegKey, name: &str| {
        let start: u64 = key.get_value("LastUsedTimeStart").unwrap_or(0);
        let stop: u64 = key.get_value("LastUsedTimeStop").unwrap_or(1);
        if start > 0 && stop == 0 {
            if let Some(app) = meeting_app(name) { if !found.contains(&app) { found.push(app); } }
        }
    };
    for name in base.enum_keys().flatten() {
        let Ok(key) = base.open_subkey(&name) else { continue };
        if name == "NonPackaged" {
            for exe in key.enum_keys().flatten() {
                if let Ok(sub) = key.open_subkey(&exe) { check(&sub, &exe); }
            }
        } else {
            check(&key, &name);
        }
    }
    found
}
#[cfg(not(windows))]
fn apps_using_microphone() -> Vec<&'static str> { vec![] }

/// Polls every few seconds and reminds once per call.
pub fn start(app: AppHandle) {
    if !cfg!(windows) { return; }
    std::thread::Builder::new().name("bolo-meeting-detect".into()).spawn(move || {
        let mut in_call = false;
        loop {
            std::thread::sleep(Duration::from_secs(4));
            let enabled = app.state::<crate::AppState>().settings.lock().unwrap_or_else(|e| e.into_inner()).meeting_reminders;
            let recording = app.state::<crate::meetings::State>().active.lock().unwrap_or_else(|e| e.into_inner()).is_some();
            let apps = if enabled && !recording { apps_using_microphone() } else { vec![] };
            let now = !apps.is_empty();
            if now && !in_call {
                let name = apps[0].to_string();
                let _ = app.emit("bolo://meeting-detected", Detected { app: name.clone() });
                use tauri_plugin_notification::NotificationExt;
                let _ = app.notification().builder()
                    .title("Meeting detected")
                    .body(format!("{name} is using your microphone. Open Bolo → Meetings to record and transcribe it."))
                    .show();
            }
            in_call = now || recording;
        }
    }).ok();
}

#[cfg(test)]
mod tests {
    use super::meeting_app;
    #[test]
    fn recognises_meeting_apps_from_registry_keys() {
        assert_eq!(meeting_app(r"C:#Users#me#AppData#Roaming#Zoom#bin#Zoom.exe"), Some("Zoom"));
        assert_eq!(meeting_app("MSTeams_8wekyb3d8bbwe"), Some("Microsoft Teams"));
        assert_eq!(meeting_app("5319275A.WhatsAppDesktop_cv1g1gvanyjgm"), Some("WhatsApp"));
        assert_eq!(meeting_app(r"C:#Program Files#Google#Chrome#Application#chrome.exe"), Some("a browser"));
        assert_eq!(meeting_app(r"C:#Users#me#AppData#Local#Bolo#bolo.exe"), None);
        assert_eq!(meeting_app("Microsoft.WindowsCamera_8wekyb3d8bbwe"), None);
        assert_eq!(meeting_app(r"C:#Games#Phasmophobia.exe"), None);
    }
}
