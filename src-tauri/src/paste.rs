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

pub fn type_text(text: &str, restore_clipboard: bool, target: &crate::style::Target) -> Result<()> {
    anyhow::ensure!(crate::hotkey::wait_modifiers_released(Duration::from_secs(2)), "Release the shortcut keys. Your text is safe in History.");
    if !crate::style::still_target(target) { return Err(anyhow!("Destination changed. Your text is safe in History.")); }
    let backup = if restore_clipboard { Some(Backup::capture()?) } else { None };
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    if let Err(e) = clipboard.set_text(text.to_string()) {
        let owned = sequence(); drop(clipboard);
        if let Some(backup) = backup { backup.restore(owned)?; }
        return Err(anyhow!("clipboard: {e}"));
    }
    let sequence = sequence();
    sleep(Duration::from_millis(40));
    let result = if crate::style::still_target(target) { press_shortcut('v') }
        else { Err(anyhow!("Destination changed. Your text is safe in History.")) };
    if let Some(backup) = backup {
        sleep(Duration::from_millis(350));
        drop(clipboard);
        backup.restore(sequence)?;
    }
    result
}

/// Copies whatever is selected in the focused app. Returns None when nothing
/// is selected. The user's clipboard is put back afterwards.
pub fn copy_selection() -> Result<Option<String>> {
    anyhow::ensure!(crate::hotkey::wait_modifiers_released(Duration::from_secs(2)), "Release the shortcut keys before editing");
    let backup = Backup::capture()?;
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let sentinel = format!("\u{2063}bolo-{}", std::process::id());
    if let Err(e) = clipboard.set_text(sentinel.clone()) {
        let owned = sequence(); drop(clipboard); backup.restore(owned)?;
        return Err(anyhow!("clipboard: {e}"));
    }
    sleep(Duration::from_millis(30));
    if let Err(e) = press_shortcut('c') {
        let owned = sequence(); drop(clipboard); backup.restore(owned)?;
        return Err(e);
    }

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
    let sequence = sequence();
    drop(clipboard);
    backup.restore(sequence)?;
    Ok(selection.filter(|s| !s.trim().is_empty()))
}

#[cfg(windows)]
fn sequence() -> u32 { unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() } }
#[cfg(not(windows))]
fn sequence() -> Option<String> { arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) }


#[cfg(windows)]
struct Backup { formats: Vec<(u32, Vec<u8>)> }
#[cfg(windows)]
struct ClipboardGuard(windows::Win32::Foundation::HWND);
#[cfg(windows)]
impl ClipboardGuard {
    fn open() -> Result<Self> {
        use windows::{core::w, Win32::{System::DataExchange::OpenClipboard, UI::WindowsAndMessaging::*}};
        unsafe {
            let window = CreateWindowExW(Default::default(), w!("STATIC"), w!("Bolo Clipboard"), Default::default(), 0,0,0,0, Some(HWND_MESSAGE), None, None, None)?;
            for _ in 0..20 {
                if OpenClipboard(Some(window)).is_ok() { return Ok(Self(window)); }
                sleep(Duration::from_millis(10));
            }
            let _ = DestroyWindow(window);
            Err(anyhow!("Clipboard is busy. Your dictation is safe in History."))
        }
    }
}
#[cfg(windows)]
impl Drop for ClipboardGuard {
    fn drop(&mut self) { unsafe {
        let _ = windows::Win32::System::DataExchange::CloseClipboard();
        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0);
    } }
}
#[cfg(windows)]
impl Backup {
    fn capture() -> Result<Self> {
        use windows::Win32::{Foundation::HGLOBAL, System::{DataExchange::*, Memory::*}};
        let _guard = ClipboardGuard::open()?;
        let mut formats = Vec::new(); let mut format = 0; let mut bytes_total = 0; let mut bitmap = false;
        unsafe {
            loop {
                format = EnumClipboardFormats(format); if format == 0 { break; }
                // Windows synthesizes a bitmap handle from DIB; store the portable DIB.
                if format == 2 || format == 9 { bitmap = true; continue; }
                anyhow::ensure!(!matches!(format, 3 | 14 | 128 | 130 | 131 | 142), "This clipboard format cannot be preserved safely. Copy text first or turn off Keep my clipboard.");
                let data = GetClipboardData(format)?;
                let mem = HGLOBAL(data.0); let size = GlobalSize(mem);
                anyhow::ensure!(size > 0, "Clipboard format cannot be copied safely");
                bytes_total += size;
                anyhow::ensure!(bytes_total <= 64 * 1024 * 1024, "Clipboard is too large to preserve safely. Your dictation remains in History.");
                let ptr = GlobalLock(mem);
                anyhow::ensure!(!ptr.is_null(), "Could not read clipboard data");
                let bytes = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
                let _ = GlobalUnlock(mem);
                formats.push((format, bytes));
            }
        }
        anyhow::ensure!(!bitmap || formats.iter().any(|(f,_)| *f == 8 || *f == 17), "Bitmap clipboard could not be preserved safely");
        Ok(Self { formats })
    }
    fn restore(self, owned_sequence: u32) -> Result<()> {
        use windows::Win32::{Foundation::{HANDLE, GlobalFree}, System::{DataExchange::*, Memory::*}};
        if sequence() != owned_sequence { return Ok(()); }
        let _guard = ClipboardGuard::open()?;
        if sequence() != owned_sequence { return Ok(()); }
        unsafe {
            EmptyClipboard()?;
            for (format, bytes) in self.formats {
                let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
                let ptr = GlobalLock(mem);
                if ptr.is_null() { let _ = GlobalFree(Some(mem)); return Err(anyhow!("Could not restore clipboard")); }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
                let _ = GlobalUnlock(mem);
                if let Err(e) = SetClipboardData(format, Some(HANDLE(mem.0))) { let _ = GlobalFree(Some(mem)); return Err(e.into()); }
            }
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    /// Opt-in: restores the user's full clipboard after exercising restoration.
    #[test]
    fn clipboard_restores_and_respects_new_copy() {
        if std::env::var("BOLO_TEST_CLIPBOARD").as_deref() != Ok("1") { return; }
        let original = Backup::capture().unwrap();
        let mut clipboard = arboard::Clipboard::new().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            clipboard.set_text("original text").unwrap();
            let backup = Backup::capture().unwrap();
            clipboard.set_text("dictation").unwrap(); let owned = sequence();
            backup.restore(owned).unwrap();
            assert_eq!(clipboard.get_text().unwrap(), "original text");
            let backup = Backup::capture().unwrap();
            clipboard.set_text("dictation").unwrap(); let owned = sequence();
            clipboard.set_text("new user copy").unwrap();
            backup.restore(owned).unwrap();
            assert_eq!(clipboard.get_text().unwrap(), "new user copy");
            clipboard.set_image(arboard::ImageData { width: 1, height: 1, bytes: std::borrow::Cow::Owned(vec![255, 0, 0, 255]) }).unwrap();
            let backup = Backup::capture().unwrap();
            clipboard.set_text("dictation").unwrap(); let owned = sequence();
            backup.restore(owned).unwrap();
            assert_eq!(clipboard.get_image().unwrap().width, 1);
            let mut files = vec![0u8; 20];
            files[0..4].copy_from_slice(&20u32.to_le_bytes());
            files[16..20].copy_from_slice(&1u32.to_le_bytes());
            for c in "C:\\Bolo-test.txt\0\0".encode_utf16() { files.extend_from_slice(&c.to_le_bytes()); }
            Backup { formats: vec![(15, files.clone())] }.restore(sequence()).unwrap();
            let backup = Backup::capture().unwrap();
            clipboard.set_text("dictation").unwrap(); let owned = sequence();
            backup.restore(owned).unwrap();
            assert!(Backup::capture().unwrap().formats.iter().any(|(f,b)| *f == 15 && *b == files));
        }));
        let owned = sequence(); drop(clipboard); original.restore(owned).unwrap();
        if let Err(panic) = result { std::panic::resume_unwind(panic); }
    }
}

// Other platforms preserve text and images; unsupported formats are left intact
// by refusing to overwrite them when preservation is requested.
#[cfg(not(windows))]
struct Backup { text: Option<String>, image: Option<arboard::ImageData<'static>> }
#[cfg(not(windows))]
impl Backup {
    fn capture() -> Result<Self> {
        let mut clipboard = arboard::Clipboard::new()?;
        let text = clipboard.get_text().ok();
        let image = clipboard.get_image().ok().map(|i| arboard::ImageData { width: i.width, height: i.height, bytes: std::borrow::Cow::Owned(i.bytes.into_owned()) });
        anyhow::ensure!(text.is_some() || image.is_some(), "Clipboard format cannot be preserved. Copy text first or turn off Keep my clipboard.");
        Ok(Self { text, image })
    }
    fn restore(self, owned: Option<String>) -> Result<()> {
        if sequence() != owned { return Ok(()); }
        let mut clipboard = arboard::Clipboard::new()?;
        if let Some(image) = self.image { clipboard.set_image(image)?; }
        else if let Some(text) = self.text { clipboard.set_text(text)?; }
        Ok(())
    }
}
