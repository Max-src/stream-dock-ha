//! Giving the pointer back after a dashboard card used the mouse on Mini
//! Monitor (`remote_mouse`). A card puts the PC's real pointer on the extra
//! display, which Windows may place out of reach of the other screens, so
//! the person's own mouse could stay stuck there. So:
//!
//! - when a card first takes the pointer, where it was is remembered (or the
//!   middle of the main screen, if it was already on the extra display);
//! - the person's own mouse (input that isn't injected, as the card's is)
//!   brings it back there at once: that first move or click is used up;
//! - so does two minutes without a card mouse command;
//! - Ctrl+Alt+Home brings it back from the extra display whoever put it
//!   there.
//!
//! A pad on screen's own Mouse on Mini Monitor (`screen_mouse`) is left
//! alone: while its pointer is inside the picture, nothing here acts.

use crate::monitor::Area;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A card that sent nothing for this long gives the pointer back.
pub const IDLE: Duration = Duration::from_secs(120);
/// The shortcut, as the settings show it.
pub const SHORTCUT: &str = "Ctrl+Alt+Home";

/// A card has the pointer on the extra display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Taken {
    /// Where it goes back to.
    pub home: (i32, i32),
    /// The extra display it's on.
    pub display: Area,
    /// The card's last mouse command.
    pub last: Instant,
}

fn contains(area: Area, at: (i32, i32)) -> bool {
    at.0 >= area.x && at.0 < area.x + area.width && at.1 >= area.y && at.1 < area.y + area.height
}

/// Where the pointer goes back to: where it was, unless that was already the
/// extra display, then the middle of the main screen.
pub fn home_for(pointer: (i32, i32), display: Area, primary_middle: (i32, i32)) -> (i32, i32) {
    if contains(display, pointer) {
        primary_middle
    } else {
        pointer
    }
}

/// What to do now about a card's pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// Leave it.
    Keep,
    /// Put the pointer back there, and forget the card had it.
    GiveBack((i32, i32)),
    /// Forget the card had it, without moving it: it's off the display already.
    Forget,
}

/// The state of things when deciding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Now {
    pub pointer: (i32, i32),
    /// A card is holding a mouse button down (a drag).
    pub card_holding: bool,
    /// A pad on screen's Mouse on Mini Monitor has the pointer.
    pub pad_on_screen_inside: bool,
    pub at: Instant,
}

/// The person's own mouse moved or pressed, at `now.pointer`.
pub fn on_own_mouse(taken: Option<Taken>, now: Now) -> Decision {
    let Some(taken) = taken else { return Decision::Keep };
    if now.pad_on_screen_inside {
        return Decision::Forget;
    }
    if !contains(taken.display, now.pointer) {
        return Decision::Forget;
    }
    if now.card_holding {
        return Decision::Keep;
    }
    Decision::GiveBack(taken.home)
}

/// Every second: the card went quiet, or the pointer left the display.
pub fn on_tick(taken: Option<Taken>, now: Now) -> Decision {
    let Some(taken) = taken else { return Decision::Keep };
    if now.pad_on_screen_inside || !contains(taken.display, now.pointer) {
        return Decision::Forget;
    }
    if now.card_holding || now.at.saturating_duration_since(taken.last) < IDLE {
        return Decision::Keep;
    }
    Decision::GiveBack(taken.home)
}

/// The shortcut: back from any extra display, whoever put it there.
pub fn on_shortcut(taken: Option<Taken>, pointer: (i32, i32), displays: &[Area], primary_middle: (i32, i32)) -> Option<(i32, i32)> {
    if !displays.iter().any(|display| contains(*display, pointer)) {
        return None;
    }
    Some(taken.map_or(primary_middle, |taken| taken.home))
}

static TAKEN: Mutex<Option<Taken>> = Mutex::new(None);
/// Whether the shortcut could be registered: None until tried.
static SHORTCUT_OK: Mutex<Option<bool>> = Mutex::new(None);

use crate::model::lock;

/// A card sent a mouse command for the extra display at `display`.
pub fn card_used(display: Area) {
    if crate::screen_mouse::pointer_inside() {
        return;
    }
    let now = Instant::now();
    let mut taken = lock(&TAKEN);
    match taken.as_mut() {
        Some(taken) => {
            taken.last = now;
            taken.display = display;
        }
        None => {
            let home = home_for(imp::pointer(), display, imp::primary_middle());
            *taken = Some(Taken { home, display, last: now });
            drop(taken);
            imp::watch();
        }
    }
}

/// The settings' line about getting the pointer back.
pub fn hint() -> String {
    match *lock(&SHORTCUT_OK) {
        Some(false) => format!(
            "Move your own mouse to bring the pointer back. {SHORTCUT} couldn't be set up: another program uses it."
        ),
        _ => format!("Move your own mouse, or press {SHORTCUT}, to bring the pointer back."),
    }
}

/// The extra displays Mini Monitor is showing now, as far as is known.
#[cfg_attr(test, allow(dead_code))]
fn extra_displays() -> Vec<Area> {
    [crate::model::PadKind::Main, crate::model::PadKind::Akp03e]
        .into_iter()
        .filter_map(|kind| crate::monitor::shown(kind)?.from)
        .collect()
}

/// Starts the thread that holds the shortcut and, while a card has the
/// pointer, watches the person's own mouse.
pub fn start() {
    imp::start();
}

#[cfg(test)]
mod imp {
    pub fn pointer() -> (i32, i32) {
        (0, 0)
    }
    pub fn primary_middle() -> (i32, i32) {
        (0, 0)
    }
    pub fn watch() {}
    pub fn start() {}
}

#[cfg(not(test))]
mod imp {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_HOME};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, PostThreadMessageW, SetCursorPos,
        SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, SM_CXSCREEN,
        SM_CYSCREEN, WH_MOUSE_LL, WM_APP, WM_HOTKEY, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN,
        WM_TIMER, WM_XBUTTONDOWN,
    };

    /// Put the hook in, or take it out.
    const WM_WATCH: u32 = WM_APP + 1;
    const WM_STOP_WATCHING: u32 = WM_APP + 2;

    static THREAD: AtomicU32 = AtomicU32::new(0);
    static STARTED: AtomicBool = AtomicBool::new(false);

    pub fn pointer() -> (i32, i32) {
        let mut at = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut at) };
        (at.x, at.y)
    }

    /// The main screen starts at 0, 0.
    pub fn primary_middle() -> (i32, i32) {
        unsafe { (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2) }
    }

    pub fn watch() {
        let thread = THREAD.load(Ordering::SeqCst);
        if thread != 0 {
            unsafe { PostThreadMessageW(thread, WM_WATCH, 0, 0) };
        }
    }

    fn now() -> Now {
        Now {
            pointer: pointer(),
            card_holding: crate::remote_mouse::holding(),
            pad_on_screen_inside: crate::screen_mouse::pointer_inside(),
            at: Instant::now(),
        }
    }

    /// Carries out a decision; says whether the card no longer has the pointer.
    fn carry_out(decision: Decision) -> bool {
        match decision {
            Decision::Keep => false,
            Decision::Forget => {
                *lock(&TAKEN) = None;
                true
            }
            Decision::GiveBack(home) => {
                *lock(&TAKEN) = None;
                unsafe { SetCursorPos(home.0, home.1) };
                crate::pad::note("gave the pointer back from Mini Monitor");
                true
            }
        }
    }

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == 0 {
            let info = &*(lparam as *const MSLLHOOKSTRUCT);
            let message = wparam as u32;
            let own = info.flags & LLMHF_INJECTED == 0;
            let counts = matches!(message, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN);
            if own && counts {
                let taken = *lock(&TAKEN);
                // Where the pointer is now: a move's own point is where it would go.
                let decision = on_own_mouse(taken, now());
                if decision != Decision::Keep {
                    let gave_back = matches!(decision, Decision::GiveBack(_));
                    carry_out(decision);
                    PostThreadMessageW(GetCurrentThreadId(), WM_STOP_WATCHING, 0, 0);
                    if gave_back {
                        // This first move or click is used up.
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    pub fn start() {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| unsafe {
            THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
            let ok = RegisterHotKey(std::ptr::null_mut(), 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_HOME as u32) != 0;
            *lock(&SHORTCUT_OK) = Some(ok);
            if !ok {
                crate::pad::note(&format!("shortcut problem: {SHORTCUT} to bring the pointer back from Mini Monitor is used by another program"));
            }
            SetTimer(std::ptr::null_mut(), 0, 1000, None);
            let mut hooked: HHOOK = std::ptr::null_mut();
            let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            // A card that took the pointer before this thread ran.
            if lock(&TAKEN).is_some() {
                PostThreadMessageW(GetCurrentThreadId(), WM_WATCH, 0, 0);
            }
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                match message.message {
                    WM_WATCH if hooked.is_null() && lock(&TAKEN).is_some() => {
                        hooked = SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0);
                        if hooked.is_null() {
                            crate::pad::note("Mini Monitor: Windows refused the hook that gives the pointer back");
                        }
                    }
                    WM_STOP_WATCHING if !hooked.is_null() && lock(&TAKEN).is_none() => {
                        UnhookWindowsHookEx(hooked);
                        hooked = std::ptr::null_mut();
                    }
                    WM_TIMER => {
                        let taken = *lock(&TAKEN);
                        if taken.is_some() && carry_out(on_tick(taken, now())) && !hooked.is_null() {
                            UnhookWindowsHookEx(hooked);
                            hooked = std::ptr::null_mut();
                        }
                    }
                    WM_HOTKEY => {
                        let taken = *lock(&TAKEN);
                        if let Some(home) = on_shortcut(taken, pointer(), &extra_displays(), primary_middle()) {
                            carry_out(Decision::GiveBack(home));
                            if !hooked.is_null() {
                                UnhookWindowsHookEx(hooked);
                                hooked = std::ptr::null_mut();
                            }
                        }
                    }
                    _ => {}
                }
                DispatchMessageW(&message);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTRA: Area = Area { x: 3440, y: 0, width: 1280, height: 720 };
    const MIDDLE: (i32, i32) = (1720, 720);

    fn now(pointer: (i32, i32), at: Instant) -> Now {
        Now { pointer, card_holding: false, pad_on_screen_inside: false, at }
    }

    fn taken(at: Instant) -> Taken {
        Taken { home: (100, 200), display: EXTRA, last: at }
    }

    #[test]
    fn the_pointer_goes_back_where_it_was_or_to_the_main_screen() {
        assert_eq!(home_for((100, 200), EXTRA, MIDDLE), (100, 200));
        assert_eq!(home_for((3500, 300), EXTRA, MIDDLE), MIDDLE);
    }

    #[test]
    fn the_persons_own_mouse_brings_it_back() {
        let t = Instant::now();
        assert_eq!(on_own_mouse(Some(taken(t)), now((3600, 400), t)), Decision::GiveBack((100, 200)));
        // Nothing taken, nothing to do.
        assert_eq!(on_own_mouse(None, now((3600, 400), t)), Decision::Keep);
    }

    #[test]
    fn not_during_a_cards_drag_nor_when_the_pad_on_screen_has_it() {
        let t = Instant::now();
        let holding = Now { card_holding: true, ..now((3600, 400), t) };
        assert_eq!(on_own_mouse(Some(taken(t)), holding), Decision::Keep);
        let pad = Now { pad_on_screen_inside: true, ..now((3600, 400), t) };
        assert_eq!(on_own_mouse(Some(taken(t)), pad), Decision::Forget);
    }

    #[test]
    fn a_pointer_already_off_the_display_is_only_forgotten() {
        let t = Instant::now();
        assert_eq!(on_own_mouse(Some(taken(t)), now((500, 500), t)), Decision::Forget);
        assert_eq!(on_tick(Some(taken(t)), now((500, 500), t)), Decision::Forget);
    }

    #[test]
    fn a_quiet_card_gives_it_back_after_two_minutes() {
        let t = Instant::now();
        assert_eq!(on_tick(Some(taken(t)), now((3600, 400), t + Duration::from_secs(60))), Decision::Keep);
        assert_eq!(on_tick(Some(taken(t)), now((3600, 400), t + IDLE)), Decision::GiveBack((100, 200)));
        let holding = Now { card_holding: true, ..now((3600, 400), t + IDLE) };
        assert_eq!(on_tick(Some(taken(t)), holding), Decision::Keep);
        assert_eq!(on_tick(None, now((3600, 400), t + IDLE)), Decision::Keep);
    }

    #[test]
    fn the_shortcut_works_from_any_extra_display_whoever_put_it_there() {
        let t = Instant::now();
        assert_eq!(on_shortcut(Some(taken(t)), (3600, 400), &[EXTRA], MIDDLE), Some((100, 200)));
        assert_eq!(on_shortcut(None, (3600, 400), &[EXTRA], MIDDLE), Some(MIDDLE));
        assert_eq!(on_shortcut(None, (500, 500), &[EXTRA], MIDDLE), None, "not on an extra display: left alone");
        assert_eq!(on_shortcut(None, (3600, 400), &[], MIDDLE), None);
    }

    #[test]
    fn the_hint_names_the_shortcut() {
        assert!(hint().contains(SHORTCUT));
    }
}
