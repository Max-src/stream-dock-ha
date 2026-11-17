//! The Record button. Windows acts on combinations like Win+Shift+S before
//! any app sees them, so while recording, and only then, keys are caught
//! before Windows gets them.
//!
//! The catching runs in a small windowless copy of this program: a keyboard
//! hook inside the app itself saw no keys while the app's window was in front,
//! while one in a separate process saw them all. It's also the safest place
//! for it: the helper ends after a few seconds at most, and when it ends for
//! any reason, Windows drops its hook.

use crate::actions::key_name;
use crate::model::lock;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_SYSKEYDOWN,
};

const LONGEST_RECORDING: Duration = Duration::from_secs(15);
const ESCAPE: u32 = 0x1B;

#[derive(Clone, Debug, PartialEq)]
pub enum Recorded {
    Keys(String),
    Cancelled,
}

/// Works out a combination from key presses and releases, as they happen.
#[derive(Default)]
struct Recorder {
    /// A lone Windows or Ctrl key is fine for "press keys", not for a shortcut.
    single_modifier_allowed: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    held: BTreeSet<u32>,
    result: Option<Recorded>,
}

impl Recorder {
    fn modifier_names(&self) -> Vec<&'static str> {
        [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect()
    }

    fn key(&mut self, vk: u32, down: bool) {
        if !down {
            self.held.remove(&vk);
            if self.result.is_none() && self.held.is_empty() {
                let names = self.modifier_names();
                if names.len() == 1 && self.single_modifier_allowed {
                    self.result = Some(Recorded::Keys(names[0].to_string()));
                } else {
                    // Only modifiers, so nothing's been chosen yet: start over.
                    (self.ctrl, self.alt, self.shift, self.win) = (false, false, false, false);
                }
            }
            return;
        }
        self.held.insert(vk);
        if self.result.is_some() {
            return;
        }
        match vk {
            0xA2 | 0xA3 | 0x11 => self.ctrl = true,
            0xA4 | 0xA5 | 0x12 => self.alt = true,
            0xA0 | 0xA1 | 0x10 => self.shift = true,
            0x5B | 0x5C => self.win = true,
            ESCAPE if self.modifier_names().is_empty() => self.result = Some(Recorded::Cancelled),
            _ => {
                let mut parts = self.modifier_names();
                let name = key_name(vk as u16);
                parts.push(&name);
                self.result = Some(Recorded::Keys(parts.join("+")));
            }
        }
    }

    /// Done once there's a result and every key is back up, so the Windows
    /// key's release can't open the Start menu.
    fn finished(&self) -> bool {
        self.result.is_some() && self.held.is_empty()
    }
}

const HELPER_OPTION: &str = "--record-keys";
const SINGLE_MODIFIER: &str = "single-modifier";

// In the helper:
static HOOK: AtomicIsize = AtomicIsize::new(0);
static STATE: Mutex<Option<Recorder>> = Mutex::new(None);

// In the app:
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HELPER: Mutex<Option<Child>> = Mutex::new(None);
static RESULT: Mutex<Option<Recorded>> = Mutex::new(None);

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(HOOK.load(Ordering::SeqCst) as HHOOK, code, wparam, lparam);
    }
    let vk = (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode;
    let down = wparam == WM_KEYDOWN as usize || wparam == WM_SYSKEYDOWN as usize;
    let mut state = lock(&STATE);
    if let Some(recorder) = state.as_mut() {
        let had_result = recorder.result.is_some();
        recorder.key(vk, down);
        if let (false, Some(result)) = (had_result, &recorder.result) {
            announce(result);
        }
        if recorder.finished() {
            PostQuitMessage(0);
        }
    }
    1 // Windows and other programs don't see this key.
}

/// Tells the app the result straight away, while keys may still be held.
fn announce(result: &Recorded) {
    let line = match result {
        Recorded::Keys(text) => format!("keys {text}\n"),
        Recorded::Cancelled => "cancelled\n".to_string(),
    };
    let mut out = std::io::stdout();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

unsafe fn run_hook() {
    let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), GetModuleHandleW(std::ptr::null()), 0);
    if hook.is_null() {
        return;
    }
    HOOK.store(hook as isize, Ordering::SeqCst);
    let started = Instant::now();
    let timer = SetTimer(std::ptr::null_mut(), 0, 100, None);
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        DispatchMessageW(&msg);
        if started.elapsed() > LONGEST_RECORDING {
            break;
        }
    }
    KillTimer(std::ptr::null_mut(), timer);
    UnhookWindowsHookEx(hook);
    HOOK.store(0, Ordering::SeqCst);
}

/// The helper's whole life, when this program is started with its option.
/// Returns true if this run is the helper.
pub fn run_as_helper() -> bool {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(HELPER_OPTION) {
        return false;
    }
    let single_modifier_allowed = args.next().as_deref() == Some(SINGLE_MODIFIER);
    *lock(&STATE) = Some(Recorder { single_modifier_allowed, ..Recorder::default() });
    unsafe { run_hook() };
    true
}

/// Reads what the helper reports.
fn parse_report(line: &str) -> Recorded {
    match line.trim().strip_prefix("keys ") {
        Some(text) if !text.is_empty() => Recorded::Keys(text.to_string()),
        _ => Recorded::Cancelled,
    }
}

/// Starts the helper, and listens for its answer on a thread of its own.
pub fn start(single_modifier_allowed: bool) {
    stop();
    let asked = std::time::Instant::now();
    while ACTIVE.load(Ordering::SeqCst) && asked.elapsed() < std::time::Duration::from_secs(1) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    *lock(&RESULT) = None;
    std::thread::spawn(move || {
        let mut command = match std::env::current_exe() {
            Ok(program) => Command::new(program),
            Err(_) => {
                *lock(&RESULT) = Some(Recorded::Cancelled);
                ACTIVE.store(false, Ordering::SeqCst);
                return;
            }
        };
        command.arg(HELPER_OPTION);
        if single_modifier_allowed {
            command.arg(SINGLE_MODIFIER);
        }
        let mut report = String::new();
        if let Ok(mut child) = command.stdin(Stdio::null()).stdout(Stdio::piped()).spawn() {
            let output = child.stdout.take();
            *lock(&HELPER) = Some(child);
            if let Some(output) = output {
                let _ = BufReader::new(output).read_line(&mut report);
            }
            // The answer can come while keys are still held, so the helper is
            // left to finish by itself.
            *lock(&RESULT) = Some(parse_report(&report));
            let helper = lock(&HELPER).take();
            if let Some(mut helper) = helper {
                let _ = helper.wait();
            }
        } else {
            *lock(&RESULT) = Some(Recorded::Cancelled);
        }
        ACTIVE.store(false, Ordering::SeqCst);
    });
}

/// Ends a recording straight away. Ending the helper removes its hook.
pub fn stop() {
    if let Some(helper) = lock(&HELPER).as_mut() {
        let _ = helper.kill();
    }
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// The recording's result, once there is one.
pub fn take() -> Option<Recorded> {
    lock(&RESULT).take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(allow: bool, keys: &[(u32, bool)]) -> Option<Recorded> {
        let mut recorder = Recorder { single_modifier_allowed: allow, ..Recorder::default() };
        for &(vk, down) in keys {
            recorder.key(vk, down);
        }
        recorder.result
    }

    fn keys(text: &str) -> Option<Recorded> {
        Some(Recorded::Keys(text.to_string()))
    }

    const WIN: u32 = 0x5B;
    const SHIFT: u32 = 0xA0;
    const CTRL: u32 = 0xA2;
    const S: u32 = 0x53;

    #[test]
    fn windows_combinations_are_recorded() {
        let presses = [(WIN, true), (SHIFT, true), (S, true)];
        assert_eq!(record(false, &presses), keys("Shift+Win+S"));
        assert_eq!(record(false, &[(0x5C, true), (0x44, true)]), keys("Win+D"));
    }

    #[test]
    fn a_plain_key_and_a_ctrl_combination_are_recorded() {
        assert_eq!(record(false, &[(0x7A, true)]), keys("F11"));
        assert_eq!(record(false, &[(CTRL, true), (0xA4, true), (0x31, true)]), keys("Ctrl+Alt+1"));
    }

    #[test]
    fn the_windows_key_alone_counts_only_where_it_can_work() {
        let tap = [(WIN, true), (WIN, false)];
        assert_eq!(record(true, &tap), keys("Win"));
        assert_eq!(record(false, &tap), None);
        // After a lone tap on a shortcut, recording carries on normally.
        assert_eq!(record(false, &[(WIN, true), (WIN, false), (0x7A, true)]), keys("F11"));
    }

    #[test]
    fn escape_cancels_but_ctrl_escape_is_a_combination() {
        assert_eq!(record(false, &[(ESCAPE, true)]), Some(Recorded::Cancelled));
        assert_eq!(record(false, &[(CTRL, true), (ESCAPE, true)]), keys("Ctrl+Escape"));
    }

    #[test]
    fn keys_stay_caught_until_all_are_released() {
        let mut recorder = Recorder::default();
        recorder.key(WIN, true);
        recorder.key(S, true);
        assert!(!recorder.finished());
        recorder.key(S, false);
        assert!(!recorder.finished());
        recorder.key(WIN, false);
        assert!(recorder.finished());
        assert_eq!(recorder.result, keys("Win+S"));
    }

    #[test]
    fn the_helpers_report_is_read_correctly() {
        assert_eq!(parse_report("keys Shift+Win+S\r\n"), Recorded::Keys("Shift+Win+S".into()));
        assert_eq!(parse_report("cancelled\n"), Recorded::Cancelled);
        assert_eq!(parse_report(""), Recorded::Cancelled);
    }

    /// Sends real key events through the real hook. F23 does nothing on its
    /// own, so nothing can happen if a key slips through.
    #[test]
    #[ignore = "catches the keyboard for a moment"]
    fn the_hook_catches_a_windows_combination() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::keybd_event;
        *lock(&STATE) = Some(Recorder::default());
        let hook = std::thread::spawn(|| unsafe { run_hook() });
        std::thread::sleep(Duration::from_millis(300));
        let press = |vk: u8, flags: u32| unsafe { keybd_event(vk, 0, flags, 0) };
        press(0x5B, 0);
        press(0x86, 0);
        press(0x86, 2);
        press(0x5B, 2);
        hook.join().unwrap();
        assert_eq!(lock(&STATE).as_ref().and_then(|r| r.result.clone()), keys("Win+F23"));
    }
}
