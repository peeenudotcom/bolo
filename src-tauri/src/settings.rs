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
    pub groq_whisper_model: String,
    /// Whisper language code, or "auto".
    pub language: String,
    /// "off", "groq" or "ollama".
    pub cleanup: String,
    pub groq_cleanup_model: String,
    pub ollama_url: String,
    pub ollama_model: String,
    /// Write Hindi in Roman script (Hinglish) instead of Devanagari. Needs cleanup on.
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
    /// Bumped when a default changes, so old settings files can be migrated.
    pub version: u32,
}

const SETTINGS_VERSION: u32 = 2;

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
            groq_whisper_model: "whisper-large-v3-turbo".into(),
            language: "en".into(),
            cleanup: "off".into(),
            groq_cleanup_model: "llama-3.3-70b-versatile".into(),
            ollama_url: "http://localhost:11434".into(),
            ollama_model: "llama3.2".into(),
            hinglish_roman: false,
            dictionary: String::new(),
            snippets: String::new(),
            restore_clipboard: true,
            autostart: false,
            app_style: true,
            spoken_formatting: true,
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
    let mut s: Settings = std::fs::read_to_string(path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // The first preview used a three-key default; move people to the new one.
    if s.hotkey == "Ctrl+Shift+Space" {
        s.hotkey = DEFAULT_HOTKEY.into();
    }
    // v0.1.0 defaulted to automatic language detection, which costs ~2 s per
    // offline dictation. Nobody could have picked it on purpose yet.
    if s.version < 2 && s.language == "auto" {
        s.language = "en".into();
    }
    s.version = SETTINGS_VERSION;
    s
}

pub fn save(app: &AppHandle, s: &Settings) -> anyhow::Result<()> {
    let p = path(app);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(s)?)?;
    Ok(())
}
