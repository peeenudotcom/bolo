use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Global shortcut. Hold to talk, or tap once to start and tap again to stop.
    pub hotkey: String,
    /// "auto" (Groq if a key is set, otherwise local), "local" or "groq".
    pub engine: String,
    /// Local Whisper model name, e.g. "base", "small", "large-v3-turbo-q5_0".
    pub local_model: String,
    pub groq_api_key: String,
    pub groq_key_saved: bool,
    pub groq_whisper_model: String,
    /// Whisper language code, or "auto".
    pub language: String,
    /// "auto" (Groq when a key is set, otherwise off), "off", "groq" or "ollama".
    pub cleanup: String,
    pub groq_cleanup_model: String,
    pub ollama_url: String,
    pub ollama_model: String,
    /// Write Hindi in Roman script (Hinglish) instead of Devanagari. On by
    /// default. Needs cleanup on.
    pub hinglish_roman: bool,
    /// One word or phrase per line. Helps spell names, brands and jargon.
    pub dictionary: String,
    /// One per line: `trigger => replacement`.
    pub snippets: String,
    pub restore_clipboard: bool,
    pub autostart: bool,
    /// Casual in chat apps, full sentences in email, literal in code editors.
    pub app_style: bool,
    /// "new line", "new paragraph", "bullet point", "heart emoji"…
    pub spoken_formatting: bool,
    pub microphone: String,
    /// session, day, or week. Audio is retained only when explicitly enabled.
    pub history_retention: String,
    pub save_audio: bool,
    pub context_enabled: bool,
    pub check_updates: bool,
    pub corrections: String,
    pub chat_style: String,
    pub email_style: String,
    pub code_style: String,
    pub other_style: String,
    /// Bumped when a default changes, so old settings files can be migrated.
    pub version: u32,
}

const SETTINGS_VERSION: u32 = 7;
pub const DEFAULT_GROQ_MODEL: &str = "qwen/qwen3.8-27b";

#[cfg(target_os = "macos")]
pub const DEFAULT_HOTKEY: &str = "hold:fn";
#[cfg(target_os = "linux")]
pub const DEFAULT_HOTKEY: &str = "hold:ctrl+super";
#[cfg(windows)]
pub const DEFAULT_HOTKEY: &str = "hold:ctrl+win";

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.into(),
            engine: "auto".into(),
            local_model: "base".into(),
            groq_api_key: String::new(),
            groq_key_saved: false,
            groq_whisper_model: "whisper-large-v3-turbo".into(),
            language: "auto".into(),
            cleanup: "auto".into(),
            groq_cleanup_model: DEFAULT_GROQ_MODEL.into(),
            ollama_url: "http://localhost:11434".into(),
            ollama_model: "llama3.2".into(),
            hinglish_roman: true,
            dictionary: String::new(),
            snippets: String::new(),
            restore_clipboard: true,
            autostart: false,
            app_style: true,
            spoken_formatting: true,
            microphone: String::new(),
            history_retention: "session".into(),
            save_audio: false,
            context_enabled: false,
            check_updates: false,
            corrections: String::new(),
            chat_style: "casual".into(),
            email_style: "formal".into(),
            code_style: "literal".into(),
            other_style: "formal".into(),
            version: SETTINGS_VERSION,
        }
    }
}

impl Settings {
    pub fn dictionary_terms(&self) -> Vec<String> {
        self.dictionary
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()
    }

    pub fn snippet_pairs(&self) -> Vec<(String, String)> {
        self.snippets
            .lines()
            .filter_map(|l| l.split_once("=>"))
            .map(|(a, b)| (a.trim().to_string(), b.trim().replace("\\n", "\n")))
            .filter(|(a, _)| !a.is_empty())
            .collect()
    }
}

fn path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_config_dir()
        .expect("no config dir")
        .join("settings.json")
}

pub fn load(app: &AppHandle) -> Settings {
    let s: Settings = std::fs::read_to_string(path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut s = migrate(s);
    // Migrate legacy plaintext only after the OS credential store accepts it.
    if !s.groq_api_key.is_empty() {
        if save(app, &s).is_err() {
            eprintln!("Could not migrate the API key to the OS credential store");
        } else { s.groq_key_saved = true; }
    } else if let Ok(entry) = keyring::Entry::new("com.peeenudotcom.bolo", "groq") {
        s.groq_api_key = entry.get_password().unwrap_or_default();
        s.groq_key_saved |= !s.groq_api_key.is_empty();
    }
    s
}

fn migrate(mut s: Settings) -> Settings {
    // The first preview used a three-key default; move people to the new one.
    if s.hotkey == "Ctrl+Shift+Space" {
        s.hotkey = DEFAULT_HOTKEY.into();
    }
    // v0.1.0 defaulted to automatic language detection, which costs ~2 s per
    // offline dictation. Nobody could have picked it on purpose yet.
    if s.version < 2 && s.language == "auto" {
        s.language = "en".into();
    }
    // v0.1.0–0.1.1 defaulted to a Groq model that has since been retired.
    if s.version < 3 && s.groq_cleanup_model == "llama-3.3-70b-versatile" {
        s.groq_cleanup_model = DEFAULT_GROQ_MODEL.into();
    }
    // Up to v0.1.2 cleanup defaulted to off, even after a Groq key was added.
    if s.version < 4 && s.cleanup == "off" {
        s.cleanup = "auto".into();
    }
    // Hinglish (Roman script) is more common than Devanagari for these users;
    // before v0.1.3 it defaulted to off.
    if s.version < 5 {
        s.hinglish_roman = true;
    }
    // The old English default also forced Hindi speech through English decoding.
    // Detect the spoken language so the script toggle can work in both directions.
    if s.version < 6 && s.language == "en" {
        s.language = "auto".into();
    }
    s.version = SETTINGS_VERSION;
    s
}

pub fn save(app: &AppHandle, s: &Settings) -> anyhow::Result<()> {
    if s.groq_api_key.is_empty() {
        if s.groq_key_saved {
            let entry = keyring::Entry::new("com.peeenudotcom.bolo", "groq")?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e.into()),
            }
        }
    } else {
        let entry = keyring::Entry::new("com.peeenudotcom.bolo", "groq")?;
        entry.set_password(&s.groq_api_key)?;
    }
    let mut disk = s.clone();
    disk.groq_api_key.clear();
    disk.groq_key_saved = !s.groq_api_key.is_empty();
    let p = path(app);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&disk)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

#[cfg(test)]
pub fn live_test_key() -> Result<String, std::env::VarError> {
    if let Ok(key) = std::env::var("BOLO_TEST_GROQ_KEY") {
        if !key.is_empty() { return Ok(key); }
    }
    if std::env::var("BOLO_LIVE_TEST").as_deref() == Ok("1") {
        return Ok(keyring::Entry::new("com.peeenudotcom.bolo", "groq").expect("credential entry")
            .get_password().expect("Live test requested but Groq credential is unavailable"));
    }
    Err(std::env::VarError::NotPresent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_language_without_reenabling_hinglish() {
        let s = migrate(Settings { version: 5, language: "en".into(), hinglish_roman: false, ..Settings::default() });
        assert_eq!(s.language, "auto");
        assert!(!s.hinglish_roman);
        assert_eq!(s.version, SETTINGS_VERSION);
        assert_eq!(Settings::default().language, "auto");
    }

    #[test]
    fn preserves_other_languages_and_new_explicit_choices() {
        for (version, language) in [(5, "hi"), (5, "pa"), (6, "en")] {
            let s = migrate(Settings { version, language: language.into(), ..Settings::default() });
            assert_eq!(s.language, language);
        }
    }
}
