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
    let release = enigo.key(SHORTCUT_MOD, Direction::Release);
    click.and(release).map_err(|e| anyhow!("keyboard: {e}"))
}

/// Sends Cmd/Ctrl+letter from a worker thread.
///
/// On macOS, enigo turns `Key::Unicode` into a keycode for the current layout
/// with `TISCopyCurrentKeyboardInputSource`/`TISGetInputSourceProperty`.
/// macOS 15 asserts those run on the main queue and kills the process
/// (EXC_BREAKPOINT in `dispatch_assert_queue`) when they don't, so the
/// keystroke is sent from the main thread and this thread waits for it.
/// Keeping `Key::Unicode` (not raw keycodes) keeps Dvorak/AZERTY layouts right.
#[cfg(target_os = "macos")]
fn send_shortcut(app: &tauri::AppHandle, letter: char) -> Result<()> {
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Arc;
    unsafe extern "C" { fn pthread_main_np() -> std::ffi::c_int; }
    if unsafe { pthread_main_np() } != 0 { return press_shortcut(letter); }

    const PENDING: u8 = 0; const RUNNING: u8 = 1; const ABANDONED: u8 = 2;
    let state = Arc::new(AtomicU8::new(PENDING));
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let job = state.clone();
    app.run_on_main_thread(move || {
        // If the caller already gave up, sending Cmd+V now could paste a
        // clipboard that has since been restored. Skip it.
        if job.compare_exchange(PENDING, RUNNING, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            let _ = tx.send(press_shortcut(letter));
        }
    }).map_err(|e| anyhow!("keyboard: {e}"))?;
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(result) => result,
        Err(_) if state.compare_exchange(PENDING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst).is_ok() =>
            Err(anyhow!("Bolo was busy and could not paste. Your text is safe in History.")),
        // The keystroke already started on the main thread; it finishes quickly.
        Err(_) => rx.recv().map_err(|_| anyhow!("keyboard: paste was interrupted"))?,
    }
}

#[cfg(not(target_os = "macos"))]
fn send_shortcut(_app: &tauri::AppHandle, letter: char) -> Result<()> { press_shortcut(letter) }

pub fn type_text(app: &tauri::AppHandle, text: &str, restore_clipboard: bool, target: &crate::style::Target) -> Result<()> {
    anyhow::ensure!(crate::hotkey::wait_modifiers_released(Duration::from_secs(2)), "Release the shortcut keys. Your text is safe in History.");
    if !crate::style::still_target(target) { return Err(anyhow!("Destination changed. Your text is safe in History.")); }
    let _operation = CLIPBOARD_OPERATION.try_lock().map_err(|_| anyhow!("Another clipboard operation is in progress. Retry from History."))?;
    let backup = if restore_clipboard { Some(Backup::capture()?) } else { None };
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let sequence = write_text(&mut clipboard, text, backup.as_ref())?;
    sleep(Duration::from_millis(40));
    let result = if !crate::style::still_target(target) {
        Err(anyhow!("Destination changed. Your text is safe in History."))
    } else if sequence != self::sequence() {
        Err(anyhow!("Clipboard changed before pasting. Your text is safe in History."))
    } else { send_shortcut(app, 'v') };
    if let Some(backup) = backup {
        sleep(Duration::from_millis(350));
        drop(clipboard);
        backup.restore(sequence)?;
    }
    result
}

/// Copies whatever is selected in the focused app. Returns None when nothing
/// is selected. The user's clipboard is put back afterwards.
pub fn copy_selection(app: &tauri::AppHandle, target: &crate::style::Target) -> Result<Option<String>> {
    anyhow::ensure!(crate::hotkey::wait_modifiers_released(Duration::from_secs(2)), "Release the shortcut keys before editing");
    let _operation = CLIPBOARD_OPERATION.try_lock().map_err(|_| anyhow!("Another clipboard operation is in progress"))?;
    let backup = Backup::capture()?;
    let mut clipboard = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard: {e}"))?;
    let sentinel = format!("\u{2063}bolo-{}", std::process::id());
    let mut owned = write_text(&mut clipboard, &sentinel, Some(&backup))?;
    sleep(Duration::from_millis(30));
    if !crate::style::still_target(target) || owned != sequence() {
        drop(clipboard);
        backup.restore(owned)?;
        return Err(anyhow!("Selection or clipboard changed. Your dictation is safe in History."));
    }
    if let Err(e) = send_shortcut(app, 'c') {
        drop(clipboard); backup.restore(owned)?;
        return Err(e);
    }

    let mut selection = None;
    for _ in 0..12 {
        sleep(Duration::from_millis(25));
        let observed = sequence();
        match clipboard.get_text() {
            Ok(t) if t != sentinel && sequence() == observed => {
                owned = observed;
                selection = Some(t);
                break;
            }
            _ => {}
        }
    }
    drop(clipboard);
    backup.restore(owned)?;
    Ok(selection.filter(|s| !s.trim().is_empty()))
}

#[cfg(windows)]
fn sequence() -> u32 { unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() } }
#[cfg(not(windows))]
fn sequence() -> Option<String> { arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) }


static CLIPBOARD_OPERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(windows)]
struct Backup { formats: Vec<(u32, Vec<u8>)>, captured_sequence: u32 }

#[cfg(windows)]
struct OwnedGlobal(windows::Win32::Foundation::HGLOBAL);
#[cfg(windows)]
impl OwnedGlobal {
    fn new(bytes: &[u8]) -> Result<Self> {
        use windows::Win32::System::Memory::*;
        anyhow::ensure!(!bytes.is_empty(), "Cannot restore an empty clipboard allocation");
        unsafe {
            let mem = Self(GlobalAlloc(GMEM_MOVEABLE, bytes.len())?);
            let ptr = GlobalLock(mem.0);
            anyhow::ensure!(!ptr.is_null(), "Could not allocate clipboard data");
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
            let _ = GlobalUnlock(mem.0);
            Ok(mem)
        }
    }
    fn publish(self, format: u32) -> Result<()> {
        unsafe { windows::Win32::System::DataExchange::SetClipboardData(format, Some(windows::Win32::Foundation::HANDLE(self.0.0)))?; }
        // Windows owns the handle only after SetClipboardData succeeds.
        std::mem::forget(self);
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for OwnedGlobal {
    fn drop(&mut self) { unsafe { let _ = windows::Win32::Foundation::GlobalFree(Some(self.0)); } }
}

#[cfg(windows)]
fn write_text(_clipboard: &mut arboard::Clipboard, text: &str, backup: Option<&Backup>) -> Result<u32> {
    let bytes: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    let memory = OwnedGlobal::new(&bytes)?;
    let guard = ClipboardGuard::open()?;
    anyhow::ensure!(backup.is_none_or(|b| b.captured_sequence == sequence()), "Clipboard changed. Retry your dictation from History.");
    unsafe { windows::Win32::System::DataExchange::EmptyClipboard()?; }
    let result = memory.publish(13);
    drop(guard);
    // CloseClipboard can synthesize additional text formats and advance the
    // sequence. Observe the completed write, not its intermediate open state.
    let owned = sequence();
    if let Err(e) = result {
        if let Some(backup) = backup { backup.restore(owned)?; }
        return Err(e);
    }
    Ok(owned)
}
#[cfg(not(windows))]
fn write_text(clipboard: &mut arboard::Clipboard, text: &str, _backup: Option<&Backup>) -> Result<Option<String>> {
    clipboard.set_text(text.to_string())?;
    Ok(Some(text.to_string()))
}

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
                windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
                format = EnumClipboardFormats(format);
                if format == 0 {
                    anyhow::ensure!(windows::Win32::Foundation::GetLastError().0 == 0, "Could not enumerate clipboard formats; clipboard left unchanged");
                    break;
                }
                // Windows synthesizes a bitmap handle from DIB; store the portable DIB.
                if format == 2 || format == 9 { bitmap = true; continue; }
                // OLE formats point at the live Office object that copied them.
                // A byte snapshot restored later breaks pasting in Office and
                // cancels Excel's copy selection; the plain text/HTML/image
                // formats saved alongside are what other apps paste anyway.
                if format >= 0xC000 {
                    let mut name = [0u16; 64];
                    let len = GetClipboardFormatNameW(format, &mut name).max(0) as usize;
                    let name = String::from_utf16_lossy(&name[..len]);
                    if matches!(name.as_str(), "DataObject" | "Ole Private Data" | "Object Descriptor" | "Link Source Descriptor" | "Embed Source" | "Link Source" | "Embedded Object") { continue; }
                }
                anyhow::ensure!(!matches!(format, 3 | 14 | 128 | 130 | 131 | 142 | 0x200..=0x3ff), "This clipboard format cannot be preserved safely. Copy text first or turn off Keep my clipboard.");
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
        Ok(Self { formats, captured_sequence: sequence() })
    }
    fn restore(&self, owned_sequence: u32) -> Result<()> {
        use windows::Win32::System::DataExchange::EmptyClipboard;
        if sequence() != owned_sequence { return Ok(()); }
        // Prepare every allocation before clearing anything. Drop frees only
        // handles not handed to Windows, including on partial failure.
        let prepared = self.formats.iter().map(|(f, b)| Ok((*f, OwnedGlobal::new(b)?)))
            .collect::<Result<Vec<_>>>()?;
        let _guard = ClipboardGuard::open()?;
        if sequence() != owned_sequence { return Ok(()); }
        unsafe { EmptyClipboard()?; }
        let mut error = None;
        for (format, memory) in prepared {
            if let Err(e) = memory.publish(format) { error = Some(e); }
        }
        if let Some(e) = error { return Err(anyhow!("Clipboard could only be partly restored: {e}")); }
        Ok(())
    }

}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn global_memory_roundtrip_and_empty_allocation_rejected() {
        assert!(OwnedGlobal::new(&[]).is_err());
        let bytes = [1u8, 2, 3, 4];
        let memory = OwnedGlobal::new(&bytes).unwrap();
        unsafe {
            let ptr = windows::Win32::System::Memory::GlobalLock(memory.0);
            assert!(!ptr.is_null());
            assert_eq!(std::slice::from_raw_parts(ptr as *const u8, bytes.len()), bytes);
            let _ = windows::Win32::System::Memory::GlobalUnlock(memory.0);
        }
        // Drop frees this unpublished allocation. This test does not touch the clipboard.
    }
    /// Opt-in: restores the user's full clipboard after exercising restoration.
    #[test]
    fn clipboard_restores_and_respects_new_copy() {
        if std::env::var("BOLO_TEST_CLIPBOARD").as_deref() != Ok("1") { return; }
        let original = Backup::capture().unwrap();
        let mut clipboard = arboard::Clipboard::new().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            clipboard.set_text("original text").unwrap();
            let backup = Backup::capture().unwrap();
            let written = write_text(&mut clipboard, "native dictation", Some(&backup)).unwrap();
            assert_eq!(written, sequence(), "Closing the clipboard must not invalidate Bolo's own write");
            sleep(Duration::from_millis(40));
            assert_eq!(written, sequence(), "Bolo's clipboard write must remain valid until paste");
            assert_eq!(clipboard.get_text().unwrap(), "native dictation");
            backup.restore(written).unwrap();
            assert_eq!(clipboard.get_text().unwrap(), "original text");
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
            Backup { formats: vec![(15, files.clone())], captured_sequence: sequence() }.restore(sequence()).unwrap();
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
