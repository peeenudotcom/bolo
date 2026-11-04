//! Puts text into whichever app has focus: copy to the clipboard, send the
//! paste shortcut, then restore what was on the clipboard before.

use anyhow::{anyhow, Result};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread::sleep;
use std::time::Duration;

#[cfg(target_os = "macos")]
const SHORTCUT_MOD: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const SHORTCUT_MOD: Key = Key::Control;

fn press_shortcut(letter: char) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| anyhow!("keyboard: {e}"))?;
    // Let go of modifiers the user might still be holding so the app sees a
    // plain Ctrl/Cmd+letter. (Never synthesize Win: a lone Win release opens Start.)
    #[cfg(target_os = "macos")]
    let held = [Key::Shift, Key::Alt, Key::Control];
    #[cfg(not(target_os = "macos"))]
    let held = [Key::Shift, Key::Alt];
    for k in held {
        let _ = enigo.key(k, Direction::Release);
    }
    enigo
        .key(SHORTCUT_MOD, Direction::Press)
        .map_err(|e| anyhow!("keyboard: {e}"))?;
    let click = enigo.key(Key::Unicode(letter), Direction::Click);
    let _ = enigo.key(SHORTCUT_MOD, Direction::Release);
    click.map_err(|e| anyhow!("keyboard: {e}"))
}

pub fn type_text(text: &str, restore_clipboard: bool) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let previous = if restore_clipboard { clipboard.get_text().ok() } else { None };
    clipboard
        .set_text(text.to_string())
        .map_err(|e| anyhow!("clipboard: {e}"))?;
    sleep(Duration::from_millis(40));
    press_shortcut('v')?;
    if let Some(prev) = previous {
        sleep(Duration::from_millis(350));
        let _ = clipboard.set_text(prev);
    }
    Ok(())
}

/// Copies whatever is selected in the focused app. Returns None when nothing
/// is selected. The user's clipboard is put back afterwards.
pub fn copy_selection() -> Result<Option<String>> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let previous = clipboard.get_text().ok();
    let sentinel = format!("\u{2063}bolo-{}", std::process::id());
    clipboard
        .set_text(sentinel.clone())
        .map_err(|e| anyhow!("clipboard: {e}"))?;
    sleep(Duration::from_millis(30));
    press_shortcut('c')?;

    let mut selection = None;
    for _ in 0..12 {
        sleep(Duration::from_millis(25));
        match clipboard.get_text() {
            Ok(t) if t != sentinel => {
                selection = Some(t);
                break;
            }
            _ => {}
        }
    }
    match previous {
        Some(p) => {
            let _ = clipboard.set_text(p);
        }
        None => {
            let _ = clipboard.clear();
        }
    }
    Ok(selection.filter(|s| !s.trim().is_empty()))
}
