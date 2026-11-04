//! Adapts output to where it's going (chat, email, code) and turns spoken
//! formatting ("new line", "heart emoji") into the real thing.

use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AppKind {
    Chat,
    Email,
    Code,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct Target {
    pub app: String,
    /// Window title, used to tell whether two dictations went to the same place.
    pub title: String,
    pub kind: AppKind,
    pub window_id: usize,
    pub edit_epoch: u64,
}

impl Default for AppKind {
    fn default() -> Self {
        AppKind::Other
    }
}

const CODE_APPS: &[&str] = &[
    "code", "cursor", "windsurf", "zed", "devenv", "idea", "pycharm", "webstorm", "rider", "clion",
    "goland", "rustrover", "studio", "sublime_text", "sublime text", "nvim", "vim", "emacs",
    "windowsterminal", "terminal", "wt", "cmd", "powershell", "pwsh", "iterm2", "iterm",
    "alacritty", "wezterm", "wezterm-gui", "kitty", "warp", "ghostty", "konsole",
    "gnome-terminal", "xterm", "tabby", "hyper",
];
const CHAT_WORDS: &[&str] = &[
    "whatsapp", "telegram", "slack", "discord", "teams", "signal", "messenger", "instagram",
    "wechat", "skype", "viber", "imessage", "messages", "google chat",
];
const EMAIL_WORDS: &[&str] = &["outlook", "thunderbird", "gmail", "inbox", "mail", "spark", "superhuman", "proton mail"];

pub fn classify(app_name: &str, exe_stem: &str, title: &str) -> AppKind {
    let app = app_name.to_lowercase();
    let exe = exe_stem.to_lowercase();
    let title = title.to_lowercase();
    if CODE_APPS.iter().any(|c| exe == *c || app == *c || app.starts_with("visual studio")) {
        return AppKind::Code;
    }
    let has_word = |hay: &str, words: &[&str]| words.iter().any(|w| contains_word(hay, w));
    if has_word(&app, CHAT_WORDS) || has_word(&exe, CHAT_WORDS) || has_word(&title, CHAT_WORDS) {
        return AppKind::Chat;
    }
    if has_word(&app, EMAIL_WORDS) || has_word(&exe, EMAIL_WORDS) || has_word(&title, EMAIL_WORDS) {
        return AppKind::Email;
    }
    AppKind::Other
}

fn contains_word(hay: &str, word: &str) -> bool {
    hay.match_indices(word).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The app the user is typing into right now.
pub fn detect() -> Target {
    match active_win_pos_rs::get_active_window() {
        Ok(w) => {
            let exe = w
                .process_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let kind = classify(&w.app_name, &exe, &w.title);
            Target { app: if w.app_name.is_empty() { exe } else { w.app_name }, title: w.title, kind,
                window_id: foreground_id(), edit_epoch: crate::hotkey::edit_epoch() }
        }
        Err(_) => Target::default(),
    }
}

fn foreground_id() -> usize {
    #[cfg(windows)]
    { unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as usize } }
    #[cfg(not(windows))]
    { 0 }
}

pub fn still_target(target: &Target) -> bool {
    let now = detect();
    matches_target(target, &now)
}

fn matches_target(target: &Target, now: &Target) -> bool {
    !target.app.is_empty() && now.app == target.app && now.title == target.title
        && now.window_id == target.window_id && now.edit_epoch == target.edit_epoch
}

pub fn preference<'a>(s: &'a crate::settings::Settings, kind: AppKind) -> &'a str {
    if !s.app_style { return "literal"; }
    match kind { AppKind::Chat => &s.chat_style, AppKind::Email => &s.email_style,
        AppKind::Code => &s.code_style, AppKind::Other => &s.other_style }
}

pub fn preference_hint(preference: &str) -> &'static str {
    match preference {
        "casual" => "Keep the speaker's words. Use natural capitalization and light punctuation, without a final full stop.",
        "lowercase" => "Keep the speaker's words. Use lowercase and light punctuation; preserve proper names and acronyms.",
        "literal" => "Keep wording, casing and technical syntax literal. Do not rewrite the speaker's phrasing.",
        _ => "Keep the speaker's words. Use sentence capitalization and complete punctuation.",
    }
}

pub fn finish_preferred(text: &str, preference: &str) -> String {
    if matches!(preference, "casual" | "lowercase" | "literal") { finish(text, AppKind::Chat) }
    else { text.trim().to_string() }
}

/// Joins a new dictation onto text already before the caret: adds the
/// separating space unless the new text starts with punctuation or a break.
pub fn join_after_text(text: &str) -> String {
    match text.chars().next() {
        Some(c) if c.is_whitespace() || ".,!?;:)]}'\"…".contains(c) => text.to_string(),
        Some(_) => format!(" {text}"),
        None => String::new(),
    }
}

/// Deterministic touch-ups that don't need an AI model.
pub fn finish(text: &str, kind: AppKind) -> String {
    let t = text.trim();
    match kind {
        AppKind::Chat | AppKind::Code if t.ends_with('.') && !t.ends_with("..") => t[..t.len() - 1].to_string(),
        _ => t.to_string(),
    }
}

// ---------- spoken formatting ----------

static EMOJI: &[(&str, &str)] = &[
    ("thumbs up", "👍"), ("thumbs down", "👎"), ("folded hands", "🙏"), ("praying hands", "🙏"),
    ("red heart", "❤️"), ("heart eyes", "😍"), ("party popper", "🎉"),
    ("heart", "❤️"), ("smiley", "😊"), ("smile", "😊"), ("smiling", "😊"), ("laughing", "😂"),
    ("laugh", "😂"), ("crying", "😢"), ("sad", "😢"), ("wink", "😉"), ("winking", "😉"),
    ("fire", "🔥"), ("party", "🎉"), ("rocket", "🚀"), ("clap", "👏"), ("clapping", "👏"),
    ("hundred", "💯"), ("100", "💯"), ("ok", "👌"), ("okay", "👌"), ("thinking", "🤔"),
    ("eyes", "👀"), ("check", "✅"), ("tick", "✅"), ("star", "⭐"), ("sparkles", "✨"),
    ("cool", "😎"), ("sunglasses", "😎"), ("kiss", "😘"), ("angry", "😠"), ("pray", "🙏"),
    ("namaste", "🙏"), ("muscle", "💪"), ("wave", "👋"), ("waving", "👋"),
];

static RULES: LazyLock<Vec<(Regex, String)>> = LazyLock::new(|| {
    let mut rules = vec![
        (Regex::new(r"(?i)[,.;]?\s*\bnew paragraph\b[,.;:]?\s*").unwrap(), "\n\n".to_string()),
        (Regex::new(r"(?i)[,.;]?\s*\b(?:new|next) line\b[,.;:]?\s*").unwrap(), "\n".to_string()),
        (Regex::new(r"(?i)[,.;]?\s*\bbullet point\b[,.;:]?\s*").unwrap(), "\n- ".to_string()),
    ];
    let names = EMOJI.iter().map(|(n, _)| regex::escape(n)).collect::<Vec<_>>().join("|");
    rules.push((Regex::new(&format!(r"(?i)\b({names})\s+emoji\b[.,]?")).unwrap(), String::new()));
    rules
});

pub fn spoken_formatting(text: &str) -> String {
    let mut out = text.to_string();
    for (re, rep) in RULES.iter() {
        if rep.is_empty() {
            out = re
                .replace_all(&out, |c: &regex::Captures| {
                    let name = c[1].to_lowercase();
                    EMOJI.iter().find(|(n, _)| *n == name).map(|(_, e)| e.to_string()).unwrap_or_default()
                })
                .into_owned();
        } else {
            out = re.replace_all(&out, rep.as_str()).into_owned();
        }
    }
    capitalize_lines(out.trim_start_matches('\n'))
}

fn capitalize_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let (prefix, rest) = line.strip_prefix("- ").map(|r| ("- ", r)).unwrap_or(("", line));
            let mut c = rest.chars();
            match c.next() {
                Some(f) => format!("{prefix}{}{}", f.to_uppercase(), c.as_str()),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_apps() {
        assert_eq!(classify("WhatsApp", "WhatsApp", ""), AppKind::Chat);
        assert_eq!(classify("Google Chrome", "chrome", "WhatsApp - Google Chrome"), AppKind::Chat);
        assert_eq!(classify("Google Chrome", "chrome", "Inbox (3) - me@gmail.com - Gmail"), AppKind::Email);
        assert_eq!(classify("Visual Studio Code", "Code", "lib.rs"), AppKind::Code);
        assert_eq!(classify("Windows Terminal", "WindowsTerminal", "pwsh"), AppKind::Code);
        assert_eq!(classify("Microsoft Word", "WINWORD", "Report.docx"), AppKind::Other);
        assert_eq!(classify("Notepad", "notepad", "Dateline notes"), AppKind::Other);
    }

    #[test]
    fn formats_spoken_commands() {
        assert_eq!(spoken_formatting("Hi team, new paragraph. thanks for coming"), "Hi team\n\nThanks for coming");
        assert_eq!(spoken_formatting("Buy milk new line eggs"), "Buy milk\nEggs");
        assert_eq!(spoken_formatting("Groceries: bullet point milk bullet point eggs"), "Groceries:\n- Milk\n- Eggs");
        assert_eq!(spoken_formatting("Great job, thumbs up emoji."), "Great job, 👍");
        assert_eq!(spoken_formatting("Love you heart emoji"), "Love you ❤️");
        assert_eq!(spoken_formatting("The sad truth"), "The sad truth");
    }

    #[test]
    fn joins_with_a_space() {
        assert_eq!(join_after_text("I'm here."), " I'm here.");
        assert_eq!(join_after_text(", and then"), ", and then");
        assert_eq!(join_after_text("\nNext line"), "\nNext line");
    }

    #[test]
    fn destination_guard_rejects_changes() {
        let target = Target { app: "Notepad".into(), title: "Note".into(), window_id: 123, edit_epoch: 8, ..Default::default() };
        assert!(matches_target(&target, &target));
        for changed in [
            Target { app: "Mail".into(), ..target.clone() },
            Target { title: "Other note".into(), ..target.clone() },
            Target { window_id: 124, ..target.clone() },
            Target { edit_epoch: 9, ..target.clone() },
        ] { assert!(!matches_target(&target, &changed)); }
        assert!(!matches_target(&Target::default(), &Target::default()));
    }

    #[test]
    fn chat_drops_final_full_stop() {
        assert_eq!(finish("See you tomorrow.", AppKind::Chat), "See you tomorrow");
        assert_eq!(finish("Wait...", AppKind::Chat), "Wait...");
        assert_eq!(finish("Done.", AppKind::Email), "Done.");
    }
}
