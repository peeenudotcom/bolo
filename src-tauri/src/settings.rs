use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Global shortcut. Hold to talk, or tap once to start and tap again to stop.
    pub hotkey: String,
    /// "auto" (OpenAI if a key is set, otherwise local), "local", or "openai".
    pub engine: String,
    /// Local Whisper model name, e.g. "base", "small", "large-v3-turbo-q5_0".
    pub local_model: String,
    pub openai_api_key: String,
    pub openai_key_saved: bool,
    /// Whisper language code, or "auto".
    pub language: String,
    /// "auto" (OpenAI when a key is set, otherwise off), "off", or "openai".
    pub cleanup: String,
    pub openai_cleanup_model: String,
    /// Write Hindi and Punjabi in Roman script instead of native scripts. On by
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
    /// Optional formatting preferences, bounded when used in a prompt.
    pub writing_preferences: String,
    pub review_before_paste: bool,
    /// Bumped when a default changes, so old settings files can be migrated.
    pub version: u32,
}

const SETTINGS_VERSION: u32 = 9;
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-6-sol";

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
            openai_api_key: String::new(),
            openai_key_saved: false,
            language: "auto".into(),
            cleanup: "auto".into(),
            openai_cleanup_model: DEFAULT_OPENAI_MODEL.into(),
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
            writing_preferences: String::new(),
            review_before_paste: false,
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

    /// Recognition also benefits from explicitly saved spellings and snippet
    /// triggers. These are hints, never instructions or replacement sentences.
    pub fn recognition_terms(&self) -> Vec<String> {
        let candidates = self.dictionary.lines().chain(
            self.corrections.lines().filter_map(|line| line.split_once("=>").map(|(_, wanted)| wanted))
        ).chain(self.snippets.lines().filter_map(|line| line.split_once("=>").map(|(trigger, _)| trigger)));
        let mut seen = std::collections::HashSet::new();
        candidates.map(str::trim)
            .filter(|term| !term.is_empty() && term.chars().count() <= 120
                && term.split_whitespace().count() <= 8
                && !term.contains(['<', '>', '\r', '\n', '\0'])
                && !term.contains("\\n"))
            .filter(|term| seen.insert(term.to_lowercase()))
            .take(100).map(str::to_string).collect()
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
    if !s.openai_api_key.is_empty() {
        if save(app, &s).is_err() {
            warning = Some("Could not move the OpenAI key to secure storage. The original key has been kept.".into());
        } else { s.openai_key_saved = true; }
    } else {
        match keyring::Entry::new("com.peeenudotcom.bolo", "openai").and_then(|e| e.get_password()) {
            Ok(key) => { s.openai_api_key = key; s.openai_key_saved = true; }
            Err(keyring::Error::NoEntry) if !s.openai_key_saved => {}
            Err(_) => { s.openai_key_saved = true; warning = Some("The saved OpenAI key is unavailable. Unlock your credential store or re-enter the key; it has not been deleted.".into()); }
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

pub fn supported_language(language: &str) -> bool {
    matches!(language, "auto" | "en" | "hi" | "pa")
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
    // Early versions defaulted cleanup to off, even after a cloud key was added.
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
    // Retired language choices fall back to mixed speech; preserve script preference.
    if !supported_language(&s.language) { s.language = "auto".into(); }
    if !matches!(s.engine.as_str(), "auto" | "local" | "openai") { s.engine = "openai".into(); }
    if !matches!(s.cleanup.as_str(), "auto" | "off" | "openai") { s.cleanup = "openai".into(); }
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
        match keyring::Entry::new("com.peeenudotcom.bolo", "openai")?.get_password() {
            Ok(v) => Ok(Some(v)), Err(keyring::Error::NoEntry) => Ok(None), Err(e) => Err(e.into())
        }
    }
    fn set(&self, value: Option<&str>) -> anyhow::Result<()> {
        let entry = keyring::Entry::new("com.peeenudotcom.bolo", "openai")?;
        match value {
            Some(v) => entry.set_password(v)?,
            None => match entry.delete_credential() { Ok(()) | Err(keyring::Error::NoEntry) => {}, Err(e) => return Err(e.into()) }
        }
        Ok(())
    }
}

pub fn resolve_key_update(new: &mut Settings, old: &Settings) -> bool {
    let remove = new.openai_api_key.is_empty();
    if new.openai_api_key == "__BOLO_SAVED_KEY__" {
        new.openai_api_key = old.openai_api_key.clone();
    }
    new.openai_key_saved = !remove && (old.openai_key_saved || !new.openai_api_key.is_empty());
    remove && old.openai_key_saved
}

fn disk_settings(s: &Settings, remove_key: bool) -> Settings {
    let mut disk = s.clone();
    disk.openai_api_key.clear();
    disk.openai_key_saved = !remove_key && (s.openai_key_saved || !s.openai_api_key.is_empty());
    disk
}

fn persist_settings(p: &std::path::Path, s: &Settings, remove_key: bool, credential: &impl Credential) -> anyhow::Result<()> {
    // Never overwrite malformed/unreadable settings, including a legacy key.
    read_settings(p)?;
    let disk = disk_settings(s, remove_key);
    if let Some(dir) = p.parent() { std::fs::create_dir_all(dir)?; }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&disk)?)?;
    let change_key = remove_key || !s.openai_api_key.is_empty();
    let previous = if change_key { credential.get()? } else { None };
    if change_key { credential.set(if remove_key { None } else { Some(&s.openai_api_key) })?; }
    if let Err(e) = std::fs::rename(&tmp, p) {
        if change_key {
            credential.set(previous.as_deref()).map_err(|_| anyhow::anyhow!("Settings could not be saved and credential rollback failed. Check your OpenAI key before continuing."))?;
        }
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
pub fn live_test_key() -> Result<String, std::env::VarError> {
    if let Ok(key) = std::env::var("BOLO_TEST_OPENAI_KEY") {
        if !key.is_empty() { return Ok(key); }
    }
    if std::env::var("BOLO_LIVE_TEST").as_deref() == Ok("1") {
        return Ok(keyring::Entry::new("com.peeenudotcom.bolo", "openai").expect("credential entry")
            .get_password().expect("Live test requested but OpenAI credential is unavailable"));
    }
    Err(std::env::VarError::NotPresent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognition_hints_include_saved_spellings_and_triggers_but_not_expansions() {
        let s = Settings {
            dictionary: "Bolo\nbolo\n <bad> ".into(),
            corrections: "sorav => Saurav\nwrong => line\\nbreak".into(),
            snippets: "my signature => Confidential office address\nMY SIGNATURE => Other text".into(),
            ..Default::default()
        };
        assert_eq!(s.recognition_terms(), vec!["Bolo", "Saurav", "my signature"]);
        let many = Settings { dictionary: (0..150).map(|i| format!("term{i}")).collect::<Vec<_>>().join("\n"), ..Default::default() };
        assert_eq!(many.recognition_terms().len(), 100);
    }

    #[test]
    fn retired_providers_migrate_without_reusing_their_secrets() {
        for engine in ["groq", "elevenlabs"] {
            let value = serde_json::json!({"version":8,"engine":engine,"cleanup":"groq","groq_api_key":"retired-secret","groq_key_saved":true,"language":"pa","hinglish_roman":true});
            let s = migrate(serde_json::from_value(value).unwrap());
            assert_eq!(s.engine, "openai");
            assert_eq!(s.cleanup, "openai");
            assert!(s.openai_api_key.is_empty());
            assert!(!s.openai_key_saved);
            assert_eq!(s.language, "pa");
            assert!(!serde_json::to_string(&s).unwrap().contains("retired-secret"));
        }
    }

    #[test]
    fn placeholder_keeps_locked_key_but_explicit_clear_removes_it() {
        let old = Settings { openai_key_saved: true, ..Default::default() };
        let mut new = Settings { openai_api_key: "__BOLO_SAVED_KEY__".into(), ..Default::default() };
        assert!(!resolve_key_update(&mut new, &old));
        assert!(new.openai_key_saved);
        assert!(new.openai_api_key.is_empty());
        new.openai_api_key.clear();
        assert!(resolve_key_update(&mut new, &old));
        assert!(!new.openai_key_saved);
    }

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
        let s = Settings { openai_key_saved: true, ..Default::default() };
        persist_settings(&p, &s, false, &credential).unwrap();
        assert_eq!(credential.value.borrow().as_deref(), Some("test-key"));
        assert!(read_settings(&p).unwrap().openai_key_saved);
        assert!(read_settings(&p).unwrap().openai_api_key.is_empty());
        assert!(persist_settings(&p, &s, true, &credential).is_err());
        assert_eq!(credential.value.borrow().as_deref(), Some("test-key"));
    }
    #[test]
    fn migration_never_serializes_the_key_and_failed_commit_rolls_back() {
        let p = test_path("migration");
        let s = Settings { openai_api_key: "new-test-key".into(), ..Default::default() };
        let credential = FakeCredential { value: std::cell::RefCell::new(Some("old-test-key".into())), locked: false, remove_tmp: Some(p.with_extension("tmp")) };
        assert!(persist_settings(&p, &s, false, &credential).is_err());
        assert_eq!(credential.value.borrow().as_deref(), Some("old-test-key"));
        let credential = FakeCredential { remove_tmp: None, ..credential };
        persist_settings(&p, &s, false, &credential).unwrap();
        let disk = std::fs::read_to_string(&p).unwrap();
        assert!(!disk.contains("new-test-key"));
        assert!(read_settings(&p).unwrap().openai_key_saved);
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
    fn retired_audio_selection_is_ignored_without_losing_preferences() {
        let old: Settings = serde_json::from_str(r#"{"version":9,"openai_speech_model":"gpt-audio-1.5","language":"pa","hinglish_roman":true}"#).unwrap();
        let migrated = migrate(old);
        assert_eq!(migrated.language, "pa");
        assert!(migrated.hinglish_roman);
        assert!(!serde_json::to_string(&migrated).unwrap().contains("openai_speech_model"));
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
    fn retired_languages_use_auto_without_changing_script_preference() {
        for language in ["ur", "bn", "ta", "te", "mr", "gu", "es", "fr", "de", "hinglish", ""] {
            assert!(!supported_language(language));
            let s = migrate(Settings { version: 7, language: language.into(), hinglish_roman: false, ..Settings::default() });
            assert_eq!(s.language, "auto");
            assert!(!s.hinglish_roman);
        }
        for language in ["auto", "en", "hi", "pa"] { assert!(supported_language(language)); }
        assert!(Settings::default().hinglish_roman);
    }

    #[test]
    fn preserves_other_languages_and_new_explicit_choices() {
        for (version, language) in [(5, "hi"), (5, "pa"), (6, "en")] {
            let s = migrate(Settings { version, language: language.into(), ..Settings::default() });
            assert_eq!(s.language, language);
        }
    }
}
