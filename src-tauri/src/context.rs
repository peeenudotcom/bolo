//! Opt-in, bounded text from the focused editable control. No screenshots or background monitoring.
#[derive(Default, Clone)]
pub struct Nearby { pub before: String, pub after: String, pub selection: bool }

pub fn read() -> Option<Nearby> {
    static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if BUSY.swap(true, std::sync::atomic::Ordering::SeqCst) { return None; }
    struct Guard;
    impl Drop for Guard { fn drop(&mut self) { BUSY.store(false, std::sync::atomic::Ordering::SeqCst); } }
    let _guard = Guard;
    #[cfg(windows)]
    { read_windows().ok() }
    #[cfg(not(windows))]
    { None }
}

#[cfg(windows)]
fn read_windows() -> windows::core::Result<Nearby> {
    use windows::{core::BOOL, Win32::{System::Com::*, UI::Accessibility::*}};
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        struct Com;
        impl Drop for Com { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
        let _com = Com;
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let element = uia.GetFocusedElement()?;
        if element.CurrentIsPassword()?.as_bool() || element.CurrentProcessId()? == std::process::id() as i32 {
            return Ok(Nearby::default());
        }
        let kind = element.CurrentControlType()?;
        let name = element.CurrentName()?.to_string().to_lowercase();
        if (kind != UIA_EditControlTypeId && kind != UIA_DocumentControlTypeId)
            || ["password", "address", "search", "card", "account", "pin", "otp"].iter().any(|word| name.contains(word)) {
            return Ok(Nearby::default());
        }
        let pattern: IUIAutomationTextPattern2 = element.GetCurrentPatternAs(UIA_TextPattern2Id)?;
        let mut active = BOOL(0);
        let caret = pattern.GetCaretRange(&mut active)?;
        if !active.as_bool() { return Ok(Nearby::default()); }
        // Never use nearby text to join into a selection being replaced.
        let selection = pattern.GetSelection()?;
        if selection.Length()? > 0 && !selection.GetElement(0)?.GetText(1)?.is_empty() { return Ok(Nearby { selection: true, ..Default::default() }); }
        let before = caret.Clone()?;
        before.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -200)?;
        let after = caret.Clone()?;
        after.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 100)?;
        Ok(Nearby { before: before.GetText(200)?.to_string(), after: after.GetText(100)?.to_string(), selection: false })
    }
}
