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

fn path(app: &AppHandle) -> anyhow::Result<PathBuf> {
    Ok(app.path().app_config_dir()?.join("settings.json"))
}

pub fn load(app: &AppHandle) -> (Settings, Option<String>) {
    let mut warning = None;
    let read = path(app).and_then(|p| read_settings(&p));
    let mut s = migrate(match read {
        Ok(s) => s,
        Err(_) => { warning = Some("Settings could not be read. The original file has been kept; settings changes are blocked until it is repaired.".into()); Settings::default() }
    });
    if !s.groq_api_key.is_empty() {
        if save(app, &s).is_err() {
            warning = Some("Could not move the Groq key to secure storage. The original key has been kept.".into());
        } else { s.groq_key_saved = true; }
    } else {
        match keyring::Entry::new("com.peeenudotcom.bolo", "groq").and_then(|e| e.get_password()) {
            Ok(key) => { s.groq_api_key = key; s.groq_key_saved = true; }
            Err(keyring::Error::NoEntry) if !s.groq_key_saved => {}
            Err(_) => { s.groq_key_saved = true; warning = Some("The saved Groq key is unavailable. Unlock your credential store or re-enter the key; it has not been deleted.".into()); }
        }
    }
    (s, warning)
}

fn read_settings(p: &std::path::Path) -> anyhow::Result<Settings> {
    match std::fs::read(p) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.into()),
    }
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
    save_with_key_removal(app, s, false)
}

pub fn save_with_key_removal(app: &AppHandle, s: &Settings, remove_key: bool) -> anyhow::Result<()> {
    persist_settings(&path(app)?, s, remove_key, &OsCredential)
}

trait Credential {
    fn get(&self) -> anyhow::Result<Option<String>>;
    fn set(&self, value: Option<&str>) -> anyhow::Result<()>;
}
struct OsCredential;
impl Credential for OsCredential {
    fn get(&self) -> anyhow::Result<Option<String>> {
        match keyring::Entry::new("com.peeenudotcom.bolo", "groq")?.get_password() {
            Ok(v) => Ok(Some(v)), Err(keyring::Error::NoEntry) => Ok(None), Err(e) => Err(e.into())
        }
    }
    fn set(&self, value: Option<&str>) -> anyhow::Result<()> {
        let entry = keyring::Entry::new("com.peeenudotcom.bolo", "groq")?;
        match value {
            Some(v) => entry.set_password(v)?,
            None => match entry.delete_credential() { Ok(()) | Err(keyring::Error::NoEntry) => {}, Err(e) => return Err(e.into()) }
        }
        Ok(())
    }
}

fn disk_settings(s: &Settings, remove_key: bool) -> Settings {
    let mut disk = s.clone();
    disk.groq_api_key.clear();
    disk.groq_key_saved = !remove_key && (s.groq_key_saved || !s.groq_api_key.is_empty());
    disk
}

fn persist_settings(p: &std::path::Path, s: &Settings, remove_key: bool, credential: &impl Credential) -> anyhow::Result<()> {
    // Never overwrite malformed/unreadable settings, including a legacy key.
    read_settings(p)?;
    let disk = disk_settings(s, remove_key);
    if let Some(dir) = p.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&disk)?)?;
    let change_key = remove_key || !s.groq_api_key.is_empty();
    let previous = if change_key { credential.get()? } else { None };
    if change_key { credential.set(if remove_key { None } else { Some(&s.groq_api_key) })?; }
    if let Err(e) = std::fs::rename(&tmp, p) {
        if change_key {
            credential.set(previous.as_deref()).map_err(|_| anyhow::anyhow!("Settings could not be saved and credential rollback failed. Check your Groq key before continuing."))?;
        }
        return Err(e.into());
    }
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

    struct FakeCredential {
        value: std::cell::RefCell<Option<String>>,
        locked: bool,
        remove_tmp: Option<PathBuf>,
    }
    impl Credential for FakeCredential {
        fn get(&self) -> anyhow::Result<Option<String>> {
            anyhow::ensure!(!self.locked, "locked");
            Ok(self.value.borrow().clone())
        }
        fn set(&self, value: Option<&str>) -> anyhow::Result<()> {
            anyhow::ensure!(!self.locked, "locked");
            *self.value.borrow_mut() = value.map(str::to_string);
            if let Some(p) = &self.remove_tmp { let _ = std::fs::remove_file(p); }
            Ok(())
        }
    }
    fn test_path(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.scratch")
            .join(format!("settings-{name}-{}-{}", std::process::id(), crate::history::now()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }
    #[test]
    fn unavailable_key_is_preserved_by_unrelated_saves() {
        let p = test_path("locked");
        let credential = FakeCredential { value: std::cell::RefCell::new(Some("test-key".into())), locked: true, remove_tmp: None };
        let s = Settings { groq_key_saved: true, ..Default::default() };
        persist_settings(&p, &s, false, &credential).unwrap();
        assert_eq!(credential.value.borrow().as_deref(), Some("test-key"));
        assert!(read_settings(&p).unwrap().groq_key_saved);
        assert!(read_settings(&p).unwrap().groq_api_key.is_empty());
        assert!(persist_settings(&p, &s, true, &credential).is_err());
        assert_eq!(credential.value.borrow().as_deref(), Some("test-key"));
    }
    #[test]
    fn migration_never_serializes_the_key_and_failed_commit_rolls_back() {
        let p = test_path("migration");
        let s = Settings { groq_api_key: "new-test-key".into(), ..Default::default() };
        let credential = FakeCredential { value: std::cell::RefCell::new(Some("old-test-key".into())), locked: false, remove_tmp: Some(p.with_extension("tmp")) };
        assert!(persist_settings(&p, &s, false, &credential).is_err());
        assert_eq!(credential.value.borrow().as_deref(), Some("old-test-key"));
        let credential = FakeCredential { remove_tmp: None, ..credential };
        persist_settings(&p, &s, false, &credential).unwrap();
        let disk = std::fs::read_to_string(&p).unwrap();
        assert!(!disk.contains("new-test-key"));
        assert!(read_settings(&p).unwrap().groq_key_saved);
        persist_settings(&p, &Settings::default(), true, &credential).unwrap();
        assert!(credential.value.borrow().is_none());
    }
    #[test]
    fn malformed_settings_are_not_overwritten() {
        let p = test_path("malformed");
        std::fs::write(&p, "{broken").unwrap();
        let credential = FakeCredential { value: std::cell::RefCell::new(Some("test-key".into())), locked: false, remove_tmp: None };
        assert!(persist_settings(&p, &Settings::default(), true, &credential).is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "{broken");
        assert_eq!(credential.value.borrow().as_deref(), Some("test-key"));
    }

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
