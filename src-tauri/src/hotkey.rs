//! Hold-to-talk shortcuts made only of modifier keys (Ctrl+Win, Fn, Right Ctrl…).
//! OS shortcut APIs can't express those, so a low-level keyboard hook feeds
//! every key event into a small state machine that emits dictation actions.
//!
//! With preset P:
//! - hold P            → dictate, release to finish
//! - P + Space         → hands-free; press P again to finish
//! - P + Alt (Option)  → command mode (edit the selection by voice)
//! - Esc while recording → cancel
//! - P + any other key → treated as a normal shortcut; recording is dropped

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, OnceLock};
use std::cell::RefCell;
use std::sync::atomic::AtomicU8;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum K {
    LCtrl,
    RCtrl,
    LAlt,
    RAlt,
    LWin,
    RWin,
    LShift,
    RShift,
    Fn,
    Space,
    Esc,
    /// A key that types a visible character (letters, digits, punctuation).
    Char,
    /// Enter or Tab: what comes next starts on fresh whitespace.
    Break,
    /// Arrows, Home/End, Page keys, Backspace, Delete: the caret moved somewhere unknown.
    Nav,
    Other,
}

impl K {
    fn is_modifier(self) -> bool {
        matches!(
            self,
            K::LCtrl | K::RCtrl | K::LAlt | K::RAlt | K::LWin | K::RWin | K::LShift | K::RShift | K::Fn
        )
    }

    /// Any non-modifier key other than Space and Esc.
    fn is_plain(self) -> bool {
        matches!(self, K::Char | K::Break | K::Nav | K::Other)
    }
}

// ---------- what's just before the caret ----------
//
// Consecutive dictations need a space between them, but Bolo can't read the
// focused app's text. Instead it remembers what it typed last and watches the
// keys pressed since: a letter typed (or Bolo's own text) means "text before
// the caret", Space/Enter/Tab mean "whitespace", and arrows, Backspace or
// shortcuts mean "unknown". A space is added only for "text".

const CARET_UNKNOWN: u8 = 0;
const CARET_SPACE: u8 = 1;
const CARET_TEXT: u8 = 2;
static CARET: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(CARET_UNKNOWN);
static EDIT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROCESSING: AtomicBool = AtomicBool::new(false);
pub fn edit_epoch() -> u64 { EDIT_EPOCH.load(Ordering::SeqCst) }
pub fn invalidate_caret() {
    CARET.store(CARET_UNKNOWN, Ordering::SeqCst);
    EDIT_EPOCH.fetch_add(1, Ordering::SeqCst);
}
pub fn set_processing(on: bool) { PROCESSING.store(on, Ordering::SeqCst); }

fn note_typed(k: K, shortcut_held: bool) {
    let v = match k {
        _ if k.is_modifier() => return,
        _ if shortcut_held => CARET_UNKNOWN,
        K::Space | K::Break => CARET_SPACE,
        K::Char => CARET_TEXT,
        _ => CARET_UNKNOWN,
    };
    CARET.store(v, Ordering::SeqCst);
    EDIT_EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// Call after Bolo types `text` into the focused app.
pub fn note_pasted(text: &str) {
    let v = if text.ends_with(char::is_whitespace) { CARET_SPACE } else { CARET_TEXT };
    CARET.store(v, Ordering::SeqCst);
}

/// True when the last thing before the caret is known to be visible text.
pub fn caret_after_text() -> bool {
    CARET.load(Ordering::SeqCst) == CARET_TEXT
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    CtrlWin,
    Fn,
    RightCtrl,
    RightAlt,
    RightCmd,
}

impl Preset {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "hold:ctrl+win" | "hold:ctrl+super" => Some(Self::CtrlWin),
            "hold:fn" => Some(Self::Fn),
            "hold:right-ctrl" => Some(Self::RightCtrl),
            "hold:right-alt" => Some(Self::RightAlt),
            "hold:right-cmd" => Some(Self::RightCmd),
            _ => None,
        }
    }

    fn active(self, p: &HashSet<K>) -> bool {
        let has = |k| p.contains(&k);
        match self {
            Self::CtrlWin => (has(K::LCtrl) || has(K::RCtrl)) && (has(K::LWin) || has(K::RWin)),
            Self::Fn => has(K::Fn),
            Self::RightCtrl => has(K::RCtrl),
            Self::RightAlt => has(K::RAlt),
            Self::RightCmd => has(K::RWin),
        }
    }

    /// The extra key that turns dictation into command mode.
    fn command_held(self, p: &HashSet<K>) -> bool {
        match self {
            Self::RightAlt => p.contains(&K::LShift) || p.contains(&K::RShift),
            _ => p.contains(&K::LAlt) || p.contains(&K::RAlt),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    CommandMode,
    HandsFree,
    Finish { command: bool },
    Cancel,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Feedback {
    /// Swallow this key event so the focused app never sees it.
    pub suppress: bool,
    /// Send a dummy key before this release so Windows doesn't open the Start
    /// menu (Win released) or the menu bar (Alt released).
    pub mask: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Holding { command: bool },
    HandsFree { command: bool },
    /// Waiting for the shortcut keys to be let go before listening again.
    Blocked,
}

#[derive(Debug)]
pub struct Machine {
    pressed: HashSet<K>,
    phase: Phase,
    preset: Option<Preset>,
    swallowed: HashSet<K>,
    chorded: HashSet<K>,
}

impl Machine {
    pub fn new() -> Self {
        Self {
            pressed: HashSet::new(),
            phase: Phase::Idle,
            preset: None,
            swallowed: HashSet::new(),
            chorded: HashSet::new(),
        }
    }

    pub fn feed(&mut self, k: K, down: bool, recording: bool, out: &mut Vec<Action>) -> Feedback {
        let mut fb = Feedback::default();
        if !down && self.swallowed.remove(&k) {
            fb.suppress = true;
        }
        let was_active = self.preset.is_some_and(|p| p.active(&self.pressed));

        if down {
            let repeat = self.pressed.contains(&k);
            if k == K::Esc && recording && !repeat {
                out.push(Action::Cancel);
                self.phase = if was_active { Phase::Blocked } else { Phase::Idle };
                self.swallowed.insert(K::Esc);
                fb.suppress = true;
                return fb;
            }
            self.pressed.insert(k);
        } else {
            self.pressed.remove(&k);
        }

        let Some(p) = self.preset else {
            return fb;
        };
        let active = p.active(&self.pressed);
        let fresh = active && !was_active;

        match self.phase {
            Phase::Idle if fresh => {
                let command = p.command_held(&self.pressed);
                self.phase = Phase::Holding { command };
                out.push(Action::Start);
                if command {
                    out.push(Action::CommandMode);
                }
            }
            Phase::Holding { command } => {
                if !active {
                    out.push(Action::Finish { command });
                    self.phase = Phase::Idle;
                } else if down && k == K::Space {
                    self.phase = Phase::HandsFree { command };
                    out.push(Action::HandsFree);
                    self.swallowed.insert(K::Space);
                    fb.suppress = true;
                } else if down && !command && p.command_held(&self.pressed) {
                    self.phase = Phase::Holding { command: true };
                    out.push(Action::CommandMode);
                } else if down && k.is_plain() {
                    // Ctrl+Win+D and friends: the user wanted a normal shortcut.
                    out.push(Action::Cancel);
                    self.phase = Phase::Blocked;
                }
            }
            Phase::HandsFree { command } => {
                if fresh {
                    out.push(Action::Finish { command });
                    self.phase = Phase::Blocked;
                } else if down && k == K::Space && active {
                    self.swallowed.insert(K::Space);
                    fb.suppress = true;
                }
            }
            Phase::Blocked if !active => self.phase = Phase::Idle,
            _ => {}
        }

        if active {
            for m in [K::LWin, K::RWin, K::LAlt, K::RAlt] {
                if self.pressed.contains(&m) {
                    self.chorded.insert(m);
                }
            }
        }
        if !down && self.chorded.remove(&k) {
            fb.mask = true;
        }
        fb
    }

    fn modifiers_down(&self) -> bool {
        self.pressed.iter().any(|k| k.is_modifier())
    }

    /// Ctrl, Alt, Win or Fn held: keys pressed now are shortcuts, not typing.
    fn shortcut_held(&self) -> bool {
        self.pressed.iter().any(|k| k.is_modifier() && !matches!(k, K::LShift | K::RShift))
    }
}

// ---------- global wiring ----------

// Callback-owned state: no application mutex can block the keyboard hook.
thread_local! { static MACHINE: RefCell<Machine> = RefCell::new(Machine::new()); }
static PRESET: AtomicU8 = AtomicU8::new(0);
static RESET: AtomicBool = AtomicBool::new(false);
static MODIFIERS_DOWN: AtomicBool = AtomicBool::new(false);
static RECORDING: AtomicBool = AtomicBool::new(false);
static TX: OnceLock<mpsc::Sender<Action>> = OnceLock::new();

pub fn set_preset(p: Option<Preset>) {
    PRESET.store(match p { None => 0, Some(Preset::CtrlWin) => 1, Some(Preset::Fn) => 2,
        Some(Preset::RightCtrl) => 3, Some(Preset::RightAlt) => 4, Some(Preset::RightCmd) => 5 }, Ordering::SeqCst);
    RESET.store(true, Ordering::SeqCst);
}

pub fn set_recording(on: bool) {
    if RECORDING.swap(on, Ordering::SeqCst) && !on { RESET.store(true, Ordering::SeqCst); }
}

pub fn wait_modifiers_released(max: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < max {
        #[cfg(windows)]
        let held = [K::LCtrl, K::RCtrl, K::LAlt, K::RAlt, K::LWin, K::RWin, K::LShift, K::RShift]
            .iter().any(|k| win::is_down(*k).unwrap_or(false));
        #[cfg(not(windows))]
        let held = MODIFIERS_DOWN.load(Ordering::SeqCst);
        if !held { return true; }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn handle(k: K, down: bool) -> Feedback {
    let mut out = Vec::with_capacity(2);
    let fb = MACHINE.with(|cell| {
        let Ok(mut m) = cell.try_borrow_mut() else { return Feedback::default(); };
        if RESET.swap(false, Ordering::SeqCst) { m.phase = Phase::Idle; }
        m.preset = match PRESET.load(Ordering::SeqCst) { 1 => Some(Preset::CtrlWin), 2 => Some(Preset::Fn),
            3 => Some(Preset::RightCtrl), 4 => Some(Preset::RightAlt), 5 => Some(Preset::RightCmd), _ => None };
        #[cfg(windows)]
        m.pressed.retain(|key| *key == k || win::is_down(*key).unwrap_or(true));
        let shortcut = m.shortcut_held();
        let processing_cancel = k == K::Esc && down && PROCESSING.load(Ordering::SeqCst);
        let fb = m.feed(k, down, RECORDING.load(Ordering::SeqCst) || processing_cancel, &mut out);
        MODIFIERS_DOWN.store(m.modifiers_down(), Ordering::SeqCst);
        if down && !fb.suppress { note_typed(k, shortcut); }
        fb
    });
    if let Some(tx) = TX.get() { for action in out { let _ = tx.send(action); } }
    fb
}

/// Installs the keyboard hook on its own thread. Actions arrive on `tx`.
pub fn start(tx: mpsc::Sender<Action>) -> Result<(), String> {
    TX.set(tx).map_err(|_| "hotkey already started".to_string())?;
    backend_start()
}

#[cfg(windows)]
fn backend_start() -> Result<(), String> {
    win::start()
}
#[cfg(target_os = "macos")]
fn backend_start() -> Result<(), String> {
    mac::start()
}
#[cfg(target_os = "linux")]
fn backend_start() -> Result<(), String> {
    linux::start()
}

// ---------- Windows: WH_KEYBOARD_LL ----------

#[cfg(windows)]
mod win {
    use super::{handle, K};
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
        WM_SYSKEYDOWN,
    };

    const MASK_VK: u16 = 0xE8; // unassigned virtual key

    fn map(vk: u32) -> K {
        match vk {
            0xA2 | 0x11 => K::LCtrl,
            0xA3 => K::RCtrl,
            0xA4 | 0x12 => K::LAlt,
            0xA5 => K::RAlt,
            0x5B => K::LWin,
            0x5C => K::RWin,
            0xA0 | 0x10 => K::LShift,
            0xA1 => K::RShift,
            0x20 => K::Space,
            0x1B => K::Esc,
            0x0D | 0x09 => K::Break,
            0x08 | 0x2E | 0x21..=0x28 => K::Nav,
            0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE2 => K::Char,
            _ => K::Other,
        }
    }

    fn vk_of(k: K) -> Option<i32> {
        Some(match k {
            K::LCtrl => 0xA2,
            K::RCtrl => 0xA3,
            K::LAlt => 0xA4,
            K::RAlt => 0xA5,
            K::LWin => 0x5B,
            K::RWin => 0x5C,
            K::LShift => 0xA0,
            K::RShift => 0xA1,
            K::Space => 0x20,
            K::Esc => 0x1B,
            K::Fn | K::Char | K::Break | K::Nav | K::Other => return None,
        })
    }

    pub fn is_down(k: K) -> Option<bool> {
        vk_of(k).map(|vk| unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0)
    }

    fn send_mask() {
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: MASK_VK, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        };
        let inputs = [key(0), key(KEYEVENTF_KEYUP)];
        unsafe {
            SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        }
    }

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let kb = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if kb.flags & LLKHF_INJECTED == 0 && kb.vkCode != MASK_VK as u32 {
                let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                let fb = handle(map(kb.vkCode), down);
                if fb.mask {
                    send_mask();
                }
                if fb.suppress {
                    return 1;
                }
            }
        }
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }

    pub fn start() -> Result<(), String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("bolo-keyboard-hook".into())
            .spawn(move || unsafe {
                let h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(std::ptr::null()), 0);
                let _mouse = SetWindowsHookExW(14, Some(mouse_hook), GetModuleHandleW(std::ptr::null()), 0);
                let _ = ready_tx.send(!h.is_null());
                if h.is_null() {
                    return;
                }
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(true) => Ok(()),
            _ => Err("Could not install the keyboard hook".into()),
        }
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // Any mouse button down can move focus or the caret; never guess spacing afterwards.
        if code == HC_ACTION as i32 && matches!(wparam as u32, 0x0201 | 0x0204 | 0x0207 | 0x020B) {
            super::invalidate_caret();
        }
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }
}

// ---------- macOS: CGEventTap (needs Accessibility permission) ----------

#[cfg(target_os = "macos")]
mod mac {
    use super::{handle, K};
    use core_foundation::runloop::CFRunLoop;
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult, EventField,
    };

    // Device-dependent modifier bits, which tell left and right apart.
    fn flag_bit(keycode: i64) -> Option<(K, u64)> {
        Some(match keycode {
            59 => (K::LCtrl, 0x0000_0001),
            62 => (K::RCtrl, 0x0000_2000),
            56 => (K::LShift, 0x0000_0002),
            60 => (K::RShift, 0x0000_0004),
            55 => (K::LWin, 0x0000_0008),
            54 => (K::RWin, 0x0000_0010),
            58 => (K::LAlt, 0x0000_0020),
            61 => (K::RAlt, 0x0000_0040),
            63 => (K::Fn, 0x0080_0000),
            _ => return None,
        })
    }

    pub fn start() -> Result<(), String> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("bolo-event-tap".into())
            .spawn(move || {
                let result = CGEventTap::with_enabled(
                    CGEventTapLocation::HID,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::Default,
                    vec![CGEventType::KeyDown, CGEventType::KeyUp, CGEventType::FlagsChanged],
                    |_proxy, etype, event| {
                        let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                        let fb = match etype {
                            CGEventType::FlagsChanged => match flag_bit(code) {
                                Some((k, bit)) => handle(k, event.get_flags().bits() & bit != 0),
                                None => return CallbackResult::Keep,
                            },
                            CGEventType::KeyDown | CGEventType::KeyUp => {
                                let k = match code {
                                    49 => K::Space,
                                    53 => K::Esc,
                                    36 | 76 | 48 => K::Break,
                                    51 | 117 | 115 | 119 | 116 | 121 | 123..=126 => K::Nav,
                                    0..=47 | 50 | 65..=92 => K::Char,
                                    _ => K::Other,
                                };
                                handle(k, matches!(etype, CGEventType::KeyDown))
                            }
                            _ => return CallbackResult::Keep,
                        };
                        if fb.suppress { CallbackResult::Drop } else { CallbackResult::Keep }
                    },
                    || {
                        let _ = ready_tx.send(true);
                        CFRunLoop::run_current();
                    },
                );
                if result.is_err() {
                    let _ = ready_tx.send(false);
                }
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(true) => Ok(()),
            _ => Err("Allow Bolo under System Settings → Privacy & Security → Accessibility, then restart Bolo".into()),
        }
    }
}

// ---------- Linux (X11): rdev, listen-only ----------

#[cfg(target_os = "linux")]
mod linux {
    use super::{handle, K};
    use rdev::{listen, EventType, Key};

    fn map(k: Key) -> K {
        match k {
            Key::ControlLeft => K::LCtrl,
            Key::ControlRight => K::RCtrl,
            Key::Alt => K::LAlt,
            Key::AltGr => K::RAlt,
            Key::MetaLeft => K::LWin,
            Key::MetaRight => K::RWin,
            Key::ShiftLeft => K::LShift,
            Key::ShiftRight => K::RShift,
            Key::Function => K::Fn,
            Key::Space => K::Space,
            Key::Escape => K::Esc,
            Key::Return | Key::KpReturn | Key::Tab => K::Break,
            Key::Backspace | Key::Delete | Key::Home | Key::End | Key::PageUp | Key::PageDown
            | Key::UpArrow | Key::DownArrow | Key::LeftArrow | Key::RightArrow => K::Nav,
            Key::KeyA | Key::KeyB | Key::KeyC | Key::KeyD | Key::KeyE | Key::KeyF | Key::KeyG
            | Key::KeyH | Key::KeyI | Key::KeyJ | Key::KeyK | Key::KeyL | Key::KeyM | Key::KeyN
            | Key::KeyO | Key::KeyP | Key::KeyQ | Key::KeyR | Key::KeyS | Key::KeyT | Key::KeyU
            | Key::KeyV | Key::KeyW | Key::KeyX | Key::KeyY | Key::KeyZ | Key::Num0 | Key::Num1
            | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8
            | Key::Num9 | Key::BackQuote | Key::Minus | Key::Equal | Key::LeftBracket
            | Key::RightBracket | Key::SemiColon | Key::Quote | Key::BackSlash | Key::IntlBackslash
            | Key::Comma | Key::Dot | Key::Slash => K::Char,
            _ => K::Other,
        }
    }

    pub fn start() -> Result<(), String> {
        std::thread::Builder::new()
            .name("bolo-rdev".into())
            .spawn(|| {
                let _ = listen(|ev| match ev.event_type {
                    EventType::KeyPress(k) => {
                        handle(map(k), true);
                    }
                    EventType::KeyRelease(k) => {
                        handle(map(k), false);
                    }
                    _ => {}
                });
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_resets_after_automatic_stop_and_swallows_escape_release() {
        // Fn has no GetAsyncKeyState mapping, so no real keyboard is exercised.
        set_preset(Some(Preset::Fn));
        handle(K::Fn, true);
        set_recording(true);
        handle(K::Space, true);
        handle(K::Space, false);
        handle(K::Fn, false);
        set_recording(false); // the two-minute limit or a failed/cancelled job
        handle(K::Fn, true);
        MACHINE.with(|m| assert!(matches!(m.borrow().phase, Phase::Holding { .. })));
        handle(K::Fn, false);
        PROCESSING.store(true, Ordering::SeqCst);
        assert!(handle(K::Esc, true).suppress);
        PROCESSING.store(false, Ordering::SeqCst);
        assert!(handle(K::Esc, false).suppress);
        set_preset(None);
    }

    fn run(m: &mut Machine, keys: &[(K, bool)], recording: bool) -> Vec<Action> {
        let mut out = Vec::new();
        for &(k, d) in keys {
            m.feed(k, d, recording, &mut out);
        }
        out
    }

    fn machine() -> Machine {
        let mut m = Machine::new();
        m.preset = Some(Preset::CtrlWin);
        m
    }

    #[test]
    fn hold_and_release_dictates() {
        let mut m = machine();
        let out = run(&mut m, &[(K::LCtrl, true), (K::LWin, true), (K::LWin, false), (K::LCtrl, false)], false);
        assert_eq!(out, vec![Action::Start, Action::Finish { command: false }]);
    }

    #[test]
    fn win_release_after_chord_is_masked() {
        let mut m = machine();
        let mut out = Vec::new();
        m.feed(K::LWin, true, false, &mut out);
        m.feed(K::LCtrl, true, false, &mut out);
        let fb = m.feed(K::LWin, false, false, &mut out);
        assert!(fb.mask);
    }

    #[test]
    fn space_goes_hands_free_and_is_swallowed() {
        let mut m = machine();
        let mut out = Vec::new();
        m.feed(K::LCtrl, true, false, &mut out);
        m.feed(K::LWin, true, false, &mut out);
        assert!(m.feed(K::Space, true, true, &mut out).suppress);
        assert!(m.feed(K::Space, false, true, &mut out).suppress);
        m.feed(K::LWin, false, true, &mut out);
        m.feed(K::LCtrl, false, true, &mut out);
        assert_eq!(out, vec![Action::Start, Action::HandsFree]);
        // Press the shortcut again to finish.
        let out = run(&mut m, &[(K::LCtrl, true), (K::LWin, true)], true);
        assert_eq!(out, vec![Action::Finish { command: false }]);
    }

    #[test]
    fn alt_switches_to_command_mode() {
        let mut m = machine();
        let out = run(
            &mut m,
            &[(K::LCtrl, true), (K::LWin, true), (K::LAlt, true), (K::LAlt, false), (K::LWin, false)],
            false,
        );
        assert_eq!(out, vec![Action::Start, Action::CommandMode, Action::Finish { command: true }]);
    }

    #[test]
    fn other_key_cancels_as_normal_shortcut() {
        let mut m = machine();
        let out = run(&mut m, &[(K::LCtrl, true), (K::LWin, true), (K::Other, true), (K::Other, false), (K::LWin, false)], false);
        assert_eq!(out, vec![Action::Start, Action::Cancel]);
    }

    #[test]
    fn escape_cancels_only_while_recording() {
        let mut m = machine();
        let mut out = Vec::new();
        assert!(!m.feed(K::Esc, true, false, &mut out).suppress);
        m.feed(K::Esc, false, false, &mut out);
        assert!(m.feed(K::Esc, true, true, &mut out).suppress);
        assert_eq!(out, vec![Action::Cancel]);
    }

    #[test]
    fn caret_tracking() {
        note_pasted("Hello there.");
        assert!(caret_after_text());
        invalidate_caret(); // clicking another field invalidates spacing
        assert!(!caret_after_text());
        note_typed(K::Space, false);
        assert!(!caret_after_text());
        note_typed(K::Char, false);
        assert!(caret_after_text());
        note_typed(K::Break, false); // Enter, e.g. message sent
        assert!(!caret_after_text());
        note_pasted("Line\n");
        assert!(!caret_after_text());
        note_typed(K::Char, true); // Ctrl+A etc. is a shortcut, not typing
        assert!(!caret_after_text());
        note_pasted("x");
        note_typed(K::LShift, false); // modifiers alone change nothing
        assert!(caret_after_text());
    }

    #[test]
    fn ctrl_alone_does_nothing() {
        let mut m = machine();
        let out = run(&mut m, &[(K::LCtrl, true), (K::Other, true), (K::Other, false), (K::LCtrl, false)], false);
        assert!(out.is_empty());
    }
}
