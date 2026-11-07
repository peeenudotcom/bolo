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

/// Window titles may change after typing (for example an unsaved document).
/// Caret tracking separately invalidates continuity on navigation or clicks.
pub fn same_window(previous: &Target, current: &Target) -> bool {
    !previous.app.is_empty() && previous.app == current.app
        && if previous.window_id != 0 && current.window_id != 0 {
            previous.window_id == current.window_id
        } else {
            previous.window_id == current.window_id && previous.title == current.title
        }
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
    if matches!(preference, "casual" | "lowercase") { finish(text, AppKind::Chat) }
    else { text.trim().to_string() }
}

/// An explicitly spoken full stop wins over a casual style's automatic removal.
pub fn finish_dictation(raw: &str, formatted: &str, cleaned: &str, preference: &str, spoken: bool) -> String {
    static FULL_STOP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:full stop|period)[.,!?;:]*\s*(?:(?:start a new paragraph|new paragraph|(?:new|next) line|line break|skip a line)[.,!?;:]*\s*)*$").expect("constant command regex"));
    let explicit_stop = spoken && FULL_STOP.find(raw).is_some_and(|m| !literal_reference(raw, m.start(), m.end()));
    let mut finished = finish_preferred(cleaned, if explicit_stop { "literal" } else { preference });
    if explicit_stop && !finished.ends_with('.') {
        finished = format!("{}.", finished.trim_end_matches(['?', '!', ',', ';', ':']));
    }
    if spoken { preserve_boundary_breaks(formatted, &finished) } else { finished }
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
        (Regex::new(r"(?i)[,;]?[ \t]*\b(?:start a new paragraph|new paragraph)\b[,.;:!?]?[ \t]*").unwrap(), "\n\n".to_string()),
        (Regex::new(r"(?i)[,;]?[ \t]*\b(?:(?:new|next) line|line break|skip a line)\b[,.;:!?]?[ \t]*").unwrap(), "\n".to_string()),
        (Regex::new(r"(?i)[,;]?[ \t]*\bbullet point\b[,.;:!?]?[ \t]*").unwrap(), "\n- ".to_string()),
    ];
    for (name, symbol) in [
        ("question mark", "?"), ("exclamation mark|exclamation point", "!"),
        ("full stop|period", "."), ("percent sign", "%"), ("comma", ","), ("semicolon", ";"), ("colon", ":"),
        ("em dash", " —"), ("ellipsis", "…"),
    ] {
        rules.push((Regex::new(&format!(r"(?i)[ \t]*\b(?:{name})\b[.,!?;:]*")).unwrap(), symbol.into()));
    }
    for (name, symbol) in [
        ("open quote|open quotes", "\""), ("open parenthesis|open paren", "("),
        ("open bracket|open square bracket", "["), ("open brace|open curly bracket", "{"),
    ] {
        rules.push((Regex::new(&format!(r"(?i)\b(?:{name})\b[.,!?;:]*[ \t]*")).unwrap(), symbol.into()));
    }
    for (name, symbol) in [
        ("close quote|close quotes|end quote", "\""), ("close parenthesis|close paren", ")"),
        ("close bracket|close square bracket", "]"), ("close brace|close curly bracket", "}"),
    ] {
        rules.push((Regex::new(&format!(r"(?i)[ \t]*\b(?:{name})\b[.,!?;:]*")).unwrap(), symbol.into()));
    }
    for (name, symbol) in [("at sign", "@"), ("underscore", "_"), ("forward slash", "/"), ("hyphen", "-"), ("decimal point", ".")] {
        rules.push((Regex::new(&format!(r"(?i)[ \t]*\b(?:{name})\b[.,!?;:]*[ \t]*")).unwrap(), symbol.into()));
    }
    for (name, symbol) in [("plus sign", "+"), ("equals sign", "="), ("hash sign", "#"), ("ampersand", "&")] {
        rules.push((Regex::new(&format!(r"(?i)\b(?:{name})\b[.,!?;:]*")).unwrap(), symbol.into()));
    }
    let names = EMOJI.iter().map(|(n, _)| regex::escape(n)).collect::<Vec<_>>().join("|");
    rules.push((Regex::new(&format!(r"(?i)\b({names})\s+emoji\b[.,]?")).unwrap(), String::new()));
    rules
});

pub fn spoken_formatting(text: &str) -> String { spoken_formatting_for(text, true) }

pub fn spoken_formatting_for(text: &str, sentence_case: bool) -> String {
    // Recognition may already punctuate immediately before the spoken command.
    // Remove that punctuation before any command expansion, so intentionally
    // repeated commands ("exclamation mark exclamation mark") still yield !!.
    static BEFORE_MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"(?i)[.,!?;:।…]+[ \t]+(question mark|exclamation mark|exclamation point|full stop|period|comma|semicolon|colon|ellipsis)\b"
    ).expect("constant punctuation regex"));
    let normalized = BEFORE_MARK.replace_all(text, |c: &regex::Captures| {
        let m = c.get(0).expect("whole match");
        if literal_reference(text, m.start(), m.end()) { m.as_str().to_string() }
        else { format!(" {}", &c[1]) }
    });
    let mut out = numbered_list(&normalized);
    for (re, rep) in RULES.iter() {
        if rep.is_empty() {
            out = re
                .replace_all(&out, |c: &regex::Captures| {
                    let m = c.get(0).expect("whole match");
                    if literal_reference(&out, m.start(), m.end()) { return m.as_str().to_string(); }
                    let name = c[1].to_lowercase();
                    EMOJI.iter().find(|(n, _)| *n == name).map(|(_, e)| e.to_string()).unwrap_or_default()
                })
                .into_owned();
        } else {
            out = re.replace_all(&out, |c: &regex::Captures| {
                let m = c.get(0).expect("whole match");
                if literal_reference(&out, m.start(), m.end()) { m.as_str().to_string() }
                else { rep.clone() }
            }).into_owned();
        }
    }
    if sentence_case { capitalize_lines(&out) } else { out }
}

// Avoid interpreting common references to punctuation as commands.
fn literal_reference(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].trim_end().split_whitespace().last().unwrap_or("").to_lowercase();
    let after = text[end..].trim_start().split_whitespace().next().unwrap_or("").to_lowercase();
    matches!(before.as_str(), "a" | "an" | "the" | "this" | "that" | "word" | "named" | "called")
        || matches!(after.as_str(), "cancer" | "punctuation")
}

fn numbered_list(text: &str) -> String {
    static ITEMS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[,;]?[ \t]*\b(numbered list|bullet point|next item|end list)\b[,.;:!?]?[ \t]*").unwrap());
    let mut number = 0;
    let mut active = false;
    ITEMS.replace_all(text, |c: &regex::Captures| {
        let m = c.get(0).unwrap();
        if literal_reference(text, m.start(), m.end()) { return m.as_str().to_string(); }
        match c[1].to_lowercase().as_str() {
            "numbered list" => { active = true; number = 1; "\n1. ".to_string() },
            "bullet point" => { active = true; number = 0; m.as_str().to_string() },
            "next item" if active && number > 0 => { number += 1; format!("\n{number}. ") },
            "next item" if active => "\n- ".to_string(),
            "end list" if active => { active = false; number = 0; "\n".to_string() },
            _ => m.as_str().to_string(),
        }
    }).into_owned()
}

pub fn list_prefix(line: &str) -> (&str, &str) {
    if let Some(rest) = line.strip_prefix("- ") { return ("- ", rest); }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with(". ") { return line.split_at(digits + 2); }
    ("", line)
}

/// Formatting-only dictations must not go through an LLM or whitespace trim.
/// Bare "heart" is a command only when it is the whole utterance.
pub fn formatting_only(text: &str) -> Option<String> {
    let name = text.trim().trim_end_matches(['.', ',', '!', '?']).to_lowercase();
    if matches!(name.as_str(), "heart" | "red heart") { return Some("❤️".into()); }
    let formatted = spoken_formatting(text);
    // Empty list markers contain a numbering digit but no dictated content.
    // Keep them local too, including the space after "1. ".
    let only_marks = formatted.split('\n').all(|line|
        !list_prefix(line).1.chars().any(char::is_alphanumeric));
    (!formatted.is_empty() && only_marks).then_some(formatted)
}

/// Cleanup and app styles trim their output. Reattach explicitly dictated
/// leading/trailing breaks, including "new paragraph" after a sentence.
pub fn preserve_boundary_breaks(formatted: &str, cleaned: &str) -> String {
    let leading = formatted.chars().take_while(|c| c.is_whitespace()).filter(|c| *c == '\n').count();
    let trailing = formatted.chars().rev().take_while(|c| c.is_whitespace()).filter(|c| *c == '\n').count();
    if leading == 0 && trailing == 0 { return cleaned.to_string(); }
    format!("{}{}{}", "\n".repeat(leading), cleaned.trim(), "\n".repeat(trailing))
}

fn capitalize_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let (prefix, rest) = list_prefix(line);
            if rest.contains('@') || rest.contains("://") { return line.to_string(); }
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
    fn joining_survives_title_changes_but_not_window_changes() {
        let previous = Target { app: "Notepad".into(), title: "Untitled".into(), window_id: 123, ..Default::default() };
        let renamed = Target { title: "*Untitled".into(), ..previous.clone() };
        assert!(same_window(&previous, &renamed));
        assert_eq!(join_after_text("Kal milte hain, please send the report."), " Kal milte hain, please send the report.");
        assert!(!same_window(&previous, &Target { window_id: 124, ..renamed.clone() }));
        assert!(!same_window(&previous, &Target { app: "Other".into(), ..renamed.clone() }));
        assert!(!same_window(&Target::default(), &Target::default()));
        assert!(!same_window(&Target { window_id: 0, ..previous }, &Target { window_id: 0, ..renamed }));
        assert_eq!(join_after_text("\nNext line"), "\nNext line");
        assert_eq!(join_after_text("?"), "?");
    }

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
    fn recognition_punctuation_does_not_duplicate_spoken_marks() {
        for (raw, expected) in [
            ("Hello, comma how are you? question mark.", "Hello, how are you?"),
            ("Great! exclamation mark", "Great!"),
            ("Wait. ellipsis", "Wait…"),
            ("Wait exclamation mark exclamation mark", "Wait!!"),
            ("Done full stop new paragraph", "Done.\n\n"),
        ] { assert_eq!(spoken_formatting(raw), expected, "{raw}"); }
        assert_eq!(spoken_formatting("Remove the comma, please."), "Remove the comma, please.");
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
    fn standalone_commands_survive_without_cleanup() {
        for (raw, expected) in [
            ("New paragraph.", "\n\n"), ("New paragraph?", "\n\n"),
            ("New line!", "\n"), ("Question mark.", "?"),
            ("Exclamation mark.", "!"), ("Full stop.", "."),
            ("Heart emoji.", "❤️"), ("Heart.", "❤️"), ("Bullet point.", "\n- "),
        ] {
            assert_eq!(formatting_only(raw).as_deref(), Some(expected), "{raw}");
            if expected.starts_with('\n') || ["?", "!", "."].contains(&expected) {
                assert_eq!(join_after_text(expected), expected);
            }
        }
        for raw in ["My heart is racing.", "Caution mark.", "Hadimoji."] {
            assert!(formatting_only(raw).is_none(), "Do not guess commands: {raw}");
        }
    }

    #[test]
    fn inline_punctuation_and_boundary_breaks_survive_styles() {
        assert_eq!(spoken_formatting("Are you coming question mark"), "Are you coming?");
        assert_eq!(spoken_formatting("Hello comma how are you question mark."), "Hello, how are you?");
        let formatted = spoken_formatting("New paragraph. hello new paragraph.");
        assert_eq!(formatted, "\n\nHello\n\n");
        for style in ["formal", "casual", "literal", "lowercase"] {
            assert_eq!(preserve_boundary_breaks(&formatted, &finish_preferred("Hello.", style)),
                if style == "casual" || style == "lowercase" { "\n\nHello\n\n" } else { "\n\nHello.\n\n" });
        }
    }

    #[test]
    fn adjacent_break_commands_do_not_erase_each_other() {
        assert_eq!(spoken_formatting("New paragraph new line hello"), "\n\n\nHello");
        assert_eq!(spoken_formatting("New paragraph bullet point milk"), "\n\n\n- Milk");
        assert_eq!(spoken_formatting("Hello. new paragraph please send the report question mark"), "Hello.\n\nPlease send the report?");
        assert_eq!(spoken_formatting("Kal meeting hai.\nNew line please send the report."), "Kal meeting hai.\n\nPlease send the report.");
    }

    #[test]
    fn explicit_full_stop_overrides_casual_style_only_when_commands_enabled() {
        assert_eq!(finish_dictation("Done full stop.", "Done.", "Done", "casual", true), "Done.");
        assert_eq!(finish_dictation("Done full stop.", "Done.", "Done.", "casual", true), "Done.");
        assert_eq!(finish_dictation("Done.", "Done.", "Done.", "casual", true), "Done");
        assert_eq!(finish_dictation("Done full stop", "Done full stop", "Done full stop.", "casual", false), "Done full stop");
    }

    #[test]
    fn empty_numbered_list_is_a_local_formatting_command() {
        assert_eq!(formatting_only("Numbered list.").as_deref(), Some("\n1. "));
        assert_eq!(formatting_only("New paragraph numbered list.").as_deref(), Some("\n\n\n1. "));
        assert!(formatting_only("Numbered list milk").is_none());
        assert!(formatting_only("The numbered list is ready").is_none());
        assert!(formatting_only("1250").is_none());
    }

    #[test]
    fn explicit_full_stop_survives_following_break_commands() {
        for (raw, breaks) in [
            ("Done full stop new paragraph", "\n\n"),
            ("Done period. Next line.", "\n"),
            ("Done full stop new line new paragraph", "\n\n\n"),
        ] {
            let formatted = spoken_formatting(raw);
            assert_eq!(finish_dictation(raw, &formatted, "Done.", "casual", true), format!("Done.{breaks}"));
        }
        let raw = "Remove the period new paragraph";
        let formatted = spoken_formatting(raw);
        assert_eq!(finish_dictation(raw, &formatted, "Remove the period.", "casual", true), "Remove the period\n\n");
    }

    #[test]
    fn bullet_lists_end_before_following_sentences() {
        assert_eq!(spoken_formatting("Shopping list colon bullet point milk next item eggs end list the total is five hundred rupees"),
            "Shopping list:\n- Milk\n- Eggs\nThe total is five hundred rupees");
        assert_eq!(spoken_formatting("Numbered list milk end list next item"), "\n1. Milk\nNext item");
        assert_eq!(spoken_formatting("The bullet point is important"), "The bullet point is important");
    }

    #[test]
    fn expanded_commands_and_numbered_lists() {
        for (raw, expected) in [
            ("Hello period", "Hello."),
            ("Hello line break world", "Hello\nWorld"),
            ("Hello start a new paragraph world", "Hello\n\nWorld"),
            ("He said open quote hello close quote", "He said \"hello\""),
            ("Note open parenthesis optional close parenthesis", "Note (optional)"),
            ("open bracket item close bracket", "[item]"),
            ("user underscore name at sign example.com", "user_name@example.com"),
            ("3 decimal point 5", "3.5"),
            ("25 percent sign", "25%"),
            ("Numbered list milk next item eggs end list thanks", "\n1. Milk\n2. Eggs\nThanks"),
        ] { assert_eq!(spoken_formatting(raw), expected, "{raw}"); }
        assert_eq!(spoken_formatting_for("camelCase new line snake_case", false), "camelCase\nsnake_case");
    }

    #[test]
    fn punctuation_references_remain_words() {
        for raw in ["Remove the comma", "A period of time", "This colon is important", "Colon cancer", "The next item is milk", "Create a numbered list", "I like the heart emoji"] {
            assert_eq!(spoken_formatting(raw), raw, "{raw}");
        }
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
        assert_eq!(finish_preferred("object.", "literal"), "object.");
    }
}
