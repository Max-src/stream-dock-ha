//! Mouse on Mini Monitor: the pad on screen's picture works like a real
//! monitor. While the pointer is over the picture, the real Windows pointer
//! is on the extra display, at the point the picture shows, and moving the
//! mouse moves it there. So everything a mouse does reaches the display just
//! as it would on a monitor: hovering, clicks with any button, double-clicks,
//! dragging windows and selecting text, menus, the wheel, and the keyboard
//! for whatever was clicked. The pointer itself shows in the picture. Leaving
//! the picture with no button held brings the pointer back to the pad.
//!
//! It uses a low-level mouse hook, which sees each move and press before any
//! window and can keep it from them by returning 1.
//!
//! Earlier, each click was copied to the display on its own and the pointer
//! jumped back. The press landed on the pad's window first, which closed the
//! display's menus, and a drag never reached the display as one press, move
//! and release.

use crate::monitor::{desktop_point, desktop_point_clamped, Area};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_XDOWN,
    MOUSEINPUT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetMessageW, KillTimer, SetCursorPos, SetTimer, SetWindowsHookExW,
    UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER, WM_XBUTTONDOWN,
    WM_XBUTTONUP,
};

/// One piece of Mini Monitor on a pad on screen, in desktop pixels, and the
/// part of the shared picture it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    /// The pad on screen's window, which must be the one under the pointer
    /// for the pointer to go in: a window over the pad keeps its own mouse.
    pub window: isize,
    /// Where it is on the desktop: left, top, right, bottom.
    pub place: [f32; 4],
    /// The part of the shared picture it shows: left, top, right, bottom.
    pub part: [f32; 4],
    /// The shared picture's size, and the display it was taken of.
    pub size: (i32, i32),
    pub from: Area,
}

impl Zone {
    fn contains(&self, x: f32, y: f32) -> bool {
        let [left, top, right, bottom] = self.place;
        x >= left && x < right && y >= top && y < bottom
    }

    /// The point of the shared picture under a desktop point, taken to the
    /// nearest edge of this piece if it's outside it.
    fn picture_point(&self, x: f32, y: f32) -> (f32, f32) {
        let [left, top, right, bottom] = self.place;
        let [part_left, part_top, part_right, part_bottom] = self.part;
        let along = (((x - left) / (right - left)).clamp(0.0, 1.0), ((y - top) / (bottom - top)).clamp(0.0, 1.0));
        (part_left + along.0 * (part_right - part_left), part_top + along.1 * (part_bottom - part_top))
    }

    /// How far a desktop point is from this piece; 0 on it.
    fn distance(&self, x: f32, y: f32) -> f32 {
        let [left, top, right, bottom] = self.place;
        let dx = (left - x).max(x - right).max(0.0);
        let dy = (top - y).max(y - bottom).max(0.0);
        dx.hypot(dy)
    }
}

/// How far inside the display's edges the pointer is kept, so a move towards
/// an edge that is also the desktop's still registers and can leave.
const INSET: i32 = 8;

/// The pointer on the extra display for a pad on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Inside {
    window: isize,
    /// Where the mouse is over the pad, in desktop pixels: it moves by the
    /// mouse's own movement, as the pointer would.
    over_pad: (f32, f32),
    /// Where the real pointer is, on the display.
    pointer: (i32, i32),
}

#[derive(Default)]
struct State {
    zones: Vec<Zone>,
    /// When the pads on screen last said where they are.
    told: Option<Instant>,
    inside: Option<Inside>,
    /// Mouse buttons held down now, from the hook's own count.
    held: u8,
}

static STATE: Mutex<State> = Mutex::new(State { zones: Vec::new(), told: None, inside: None, held: 0 });
static HOOKED: AtomicBool = AtomicBool::new(false);
/// When Windows last refused the hook, so it isn't asked again every frame.
static REFUSED: Mutex<Option<Instant>> = Mutex::new(None);
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// The thread that holds the hook, told to remove it when it's not needed.
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
/// If the pads on screen stop saying where they are for this long, as when
/// the app is stuck, the pointer is given back.
const GIVE_BACK_AFTER: Duration = Duration::from_secs(2);

use crate::model::lock;

/// Whether the pointer is on the display for a pad on screen now.
pub fn pointer_inside() -> bool {
    lock(&STATE).inside.is_some()
}

/// Where Mini Monitor takes the mouse now, from the pads on screen drawn this
/// frame. The hook goes in when there's somewhere, once, and comes out as soon
/// as there's nowhere.
pub fn set_zones(zones: Vec<Zone>) {
    let wanted = !zones.is_empty();
    let give_back = {
        let mut state = lock(&STATE);
        state.zones = zones;
        state.told = Some(Instant::now());
        // A pad that closed, or lost Mini Monitor, gives the pointer back.
        let gone = state.inside.is_some_and(|inside| !state.zones.iter().any(|zone| zone.window == inside.window));
        if gone { state.inside.take() } else { None }
    };
    if let Some(inside) = give_back {
        put_pointer(inside.over_pad);
    }
    let refused_lately = lock(&REFUSED).is_some_and(|at| at.elapsed() < RETRY_AFTER);
    if wanted && !refused_lately && !HOOKED.swap(true, Ordering::SeqCst) {
        start();
    } else if !wanted && HOOKED.load(Ordering::SeqCst) {
        stop();
    }
}

/// Asks the hook's thread to take the hook out now.
fn stop() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
    let thread = HOOK_THREAD.load(Ordering::SeqCst);
    if thread != 0 {
        unsafe { PostThreadMessageW(thread, WM_QUIT, 0, 0) };
    }
}

fn put_pointer(at: (f32, f32)) {
    unsafe { SetCursorPos(at.0.round() as i32, at.1.round() as i32) };
}

/// What the hook does with a move or a press.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// Leave it to Windows.
    Pass,
    /// Put the pointer here and keep the move from every window.
    Move((i32, i32)),
    /// The pointer goes onto the display here, then presses this button there.
    MoveAndPress((i32, i32), u32),
}

/// The bit for a button message in the held count, and whether it's a press.
fn button_of(message: u32, data: u32) -> Option<(u8, bool)> {
    let x_button = if (data >> 16) & 0xffff == 2 { 16 } else { 8 };
    match message {
        WM_LBUTTONDOWN => Some((1, true)),
        WM_LBUTTONUP => Some((1, false)),
        WM_RBUTTONDOWN => Some((2, true)),
        WM_RBUTTONUP => Some((2, false)),
        WM_MBUTTONDOWN => Some((4, true)),
        WM_MBUTTONUP => Some((4, false)),
        WM_XBUTTONDOWN => Some((x_button, true)),
        WM_XBUTTONUP => Some((x_button, false)),
        _ => None,
    }
}

/// The input that presses a button, from its bit.
fn press_flags(bit: u8) -> (u32, u32) {
    match bit {
        1 => (MOUSEEVENTF_LEFTDOWN, 0),
        2 => (MOUSEEVENTF_RIGHTDOWN, 0),
        4 => (MOUSEEVENTF_MIDDLEDOWN, 0),
        8 => (MOUSEEVENTF_XDOWN, 1),
        _ => (MOUSEEVENTF_XDOWN, 2),
    }
}

/// Decides what happens to one move or press, and keeps track. `at` is where
/// Windows would put the pointer, and `under` the window there.
fn step(state: &mut State, message: u32, data: u32, at: (f32, f32), under: Option<isize>) -> Step {
    let button = button_of(message, data);
    let was_held = state.held;
    if let Some((bit, down)) = button {
        if down {
            state.held |= bit;
        } else {
            state.held &= !bit;
        }
    }
    match state.inside {
        // On the display, buttons and the wheel already go where the pointer is.
        Some(mut inside) if message == WM_MOUSEMOVE => {
            let delta = (at.0 - inside.pointer.0 as f32, at.1 - inside.pointer.1 as f32);
            inside.over_pad = (inside.over_pad.0 + delta.0, inside.over_pad.1 + delta.1);
            let zones: Vec<&Zone> = state.zones.iter().filter(|zone| zone.window == inside.window).collect();
            let Some(bounds) = bounds(&zones) else {
                state.inside = None;
                return Step::Move(rounded(inside.over_pad));
            };
            let on_picture = inside_bounds(bounds, inside.over_pad);
            if !on_picture && state.held == 0 {
                // Off the picture with nothing held: back to the pad.
                state.inside = None;
                return Step::Move(rounded(inside.over_pad));
            }
            // A drag stays on the display, however far the mouse goes.
            // max() so a pad under a pixel across can't make an impossible range.
            let (right, bottom) = ((bounds[2] - 1.0).max(bounds[0]), (bounds[3] - 1.0).max(bounds[1]));
            inside.over_pad = (inside.over_pad.0.clamp(bounds[0], right), inside.over_pad.1.clamp(bounds[1], bottom));
            let nearest = zones.iter().min_by(|a, b| a.distance(inside.over_pad.0, inside.over_pad.1).total_cmp(&b.distance(inside.over_pad.0, inside.over_pad.1)));
            let Some(zone) = nearest else { return Step::Pass };
            let picture = zone.picture_point(inside.over_pad.0, inside.over_pad.1);
            inside.pointer = desktop_point_clamped(zone.from, zone.size, picture.0, picture.1, INSET);
            state.inside = Some(inside);
            Step::Move(inside.pointer)
        }
        Some(_) => Step::Pass,
        None => {
            // In only from a move or a press on the picture itself, with no
            // button already held, as when dragging something across the pad.
            let pressing = button.is_some_and(|(_, down)| down);
            if (message != WM_MOUSEMOVE && !pressing) || was_held != 0 {
                return Step::Pass;
            }
            let zone = state.zones.iter().find(|zone| under == Some(zone.window) && zone.contains(at.0, at.1));
            let Some(zone) = zone else { return Step::Pass };
            let picture = zone.picture_point(at.0, at.1);
            // The bars round the display show nothing.
            if desktop_point(zone.from, zone.size, picture.0, picture.1).is_none() {
                return Step::Pass;
            }
            let pointer = desktop_point_clamped(zone.from, zone.size, picture.0, picture.1, INSET);
            state.inside = Some(Inside { window: zone.window, over_pad: at, pointer });
            match button {
                Some((bit, true)) => Step::MoveAndPress(pointer, bit as u32),
                _ => Step::Move(pointer),
            }
        }
    }
}

fn bounds(zones: &[&Zone]) -> Option<[f32; 4]> {
    zones.iter().map(|zone| zone.place).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

fn inside_bounds(bounds: [f32; 4], at: (f32, f32)) -> bool {
    at.0 >= bounds[0] && at.0 < bounds[2] && at.1 >= bounds[1] && at.1 < bounds[3]
}

fn rounded(at: (f32, f32)) -> (i32, i32) {
    (at.0.round() as i32, at.1.round() as i32)
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == 0 {
        let info = &*(lparam as *const MSLLHOOKSTRUCT);
        // Input this app sends itself is let through untouched.
        if info.flags & LLMHF_INJECTED == 0 {
            let message = wparam as u32;
            let at = (info.pt.x as f32, info.pt.y as f32);
            let mut state = lock(&STATE);
            let entering = state.inside.is_none() && !state.zones.is_empty();
            let under = if entering { root_under(info.pt) } else { None };
            match step(&mut state, message, info.mouseData, at, under) {
                Step::Pass => {}
                Step::Move(to) => {
                    drop(state);
                    // Moved at once, so the next move counts from here.
                    SetCursorPos(to.0, to.1);
                    return 1;
                }
                Step::MoveAndPress(to, bit) => {
                    drop(state);
                    SetCursorPos(to.0, to.1);
                    // The press itself, now on the display.
                    let (flags, data) = press_flags(bit as u8);
                    let input = INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
                    };
                    SendInput(1, &input, std::mem::size_of::<INPUT>() as i32);
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

/// The top window under a desktop point.
fn root_under(at: POINT) -> Option<isize> {
    unsafe {
        let window: HWND = WindowFromPoint(at);
        (!window.is_null()).then(|| GetAncestor(window, GA_ROOT) as isize)
    }
}

/// The hook needs a thread with a message loop of its own. It ends when told
/// to, and every second it checks whether the pads on screen have gone
/// quiet, in which case it gives the pointer back and ends too.
fn start() {
    std::thread::spawn(|| unsafe {
        let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let handle = SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0);
        if handle.is_null() {
            crate::pad::note("Mouse on Mini Monitor: Windows refused the mouse hook");
            *lock(&REFUSED) = Some(Instant::now());
            HOOKED.store(false, Ordering::SeqCst);
            return;
        }
        HOOK_THREAD.store(windows_sys::Win32::System::Threading::GetCurrentThreadId(), Ordering::SeqCst);
        let timer = SetTimer(std::ptr::null_mut(), 0, 1000, None);
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_TIMER {
                let (empty, give_back) = {
                    let mut state = lock(&STATE);
                    let quiet = state.told.is_none_or(|told| told.elapsed() >= GIVE_BACK_AFTER);
                    let give_back = if quiet || state.zones.is_empty() { state.inside.take() } else { None };
                    (state.zones.is_empty() || quiet, give_back)
                };
                if let Some(inside) = give_back {
                    put_pointer(inside.over_pad);
                }
                if empty {
                    break;
                }
            }
            DispatchMessageW(&message);
        }
        HOOK_THREAD.store(0, Ordering::SeqCst);
        KillTimer(std::ptr::null_mut(), timer);
        UnhookWindowsHookEx(handle);
        let give_back = lock(&STATE).inside.take();
        if let Some(inside) = give_back {
            put_pointer(inside.over_pad);
        }
        HOOKED.store(false, Ordering::SeqCst);
        // Zones that came while it was closing start it again.
        let wanted = !lock(&STATE).zones.is_empty();
        if wanted && !HOOKED.swap(true, Ordering::SeqCst) {
            start();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> Zone {
        // A pad on screen's whole picture at 100,100 to 525,355 on the desktop,
        // showing a 1280 × 720 display to the right of a 1920-wide one. The
        // display fills 425 × 239 of the 425 × 255 picture, with 8-pixel bars.
        Zone {
            window: 7,
            place: [100.0, 100.0, 525.0, 355.0],
            part: [0.0, 0.0, 425.0, 255.0],
            size: (425, 255),
            from: Area { x: 1920, y: 0, width: 1280, height: 720 },
        }
    }

    fn state() -> State {
        State { zones: vec![zone()], told: Some(Instant::now()), ..State::default() }
    }

    fn near(a: (i32, i32), b: (i32, i32)) -> bool {
        (a.0 - b.0).abs() <= 3 && (a.1 - b.1).abs() <= 3
    }

    #[test]
    fn the_pointer_goes_onto_the_display_over_the_picture() {
        let mut state = state();
        let Step::Move(to) = step(&mut state, WM_MOUSEMOVE, 0, (312.5, 227.5), Some(7)) else { panic!() };
        assert!(near(to, (2560, 360)), "{to:?}");
        // On the display, presses and the wheel go to it as they are.
        assert_eq!(step(&mut state, WM_LBUTTONDOWN, 0, (2560.0, 360.0), None), Step::Pass);
        assert_eq!(step(&mut state, 0x020A, 0, (2560.0, 360.0), None), Step::Pass);
    }

    #[test]
    fn a_drag_moves_the_pointer_on_the_display_and_stays_there() {
        let mut state = state();
        let Step::Move(start) = step(&mut state, WM_MOUSEMOVE, 0, (312.5, 227.5), Some(7)) else { panic!() };
        assert_eq!(step(&mut state, WM_LBUTTONDOWN, 0, (start.0 as f32, start.1 as f32), None), Step::Pass);
        // The mouse moves 10 pixels right: 10 on the pad, about 30 on the display.
        let Step::Move(moved) = step(&mut state, WM_MOUSEMOVE, 0, (start.0 as f32 + 10.0, start.1 as f32), None) else { panic!() };
        assert!(near(moved, (start.0 + 30, start.1)), "{start:?} to {moved:?}");
        // Far past the picture while held: it stays at the display's edge.
        let Step::Move(held) = step(&mut state, WM_MOUSEMOVE, 0, (moved.0 as f32 + 900.0, moved.1 as f32), None) else { panic!() };
        assert_eq!(held.0, 1920 + 1280 - 1 - INSET);
        assert!(state.inside.is_some());
        // Let go, then move off the picture: back to the pad, beside it.
        assert_eq!(step(&mut state, WM_LBUTTONUP, 0, (held.0 as f32, held.1 as f32), None), Step::Pass);
        let Step::Move(back) = step(&mut state, WM_MOUSEMOVE, 0, (held.0 as f32 + 5.0, held.1 as f32), None) else { panic!() };
        assert!(state.inside.is_none());
        assert!(back.0 >= 525 && back.0 < 535, "{back:?}");
    }

    #[test]
    fn a_press_on_the_picture_without_a_move_first_still_reaches_the_display() {
        let mut state = state();
        let Step::MoveAndPress(to, bit) = step(&mut state, WM_RBUTTONDOWN, 0, (312.5, 227.5), Some(7)) else { panic!() };
        assert!(near(to, (2560, 360)) && bit == 2, "{to:?} {bit}");
        assert_eq!(state.held, 2);
    }

    #[test]
    fn everything_else_is_left_alone() {
        let mut state = state();
        // Off the picture, another window over the pad, or the bars round the display.
        assert_eq!(step(&mut state, WM_MOUSEMOVE, 0, (90.0, 227.5), Some(7)), Step::Pass);
        assert_eq!(step(&mut state, WM_MOUSEMOVE, 0, (312.5, 227.5), Some(8)), Step::Pass);
        assert_eq!(step(&mut state, WM_MOUSEMOVE, 0, (312.5, 102.0), Some(7)), Step::Pass);
        // Something dragged across the pad from elsewhere stays where it is.
        assert_eq!(step(&mut state, WM_LBUTTONDOWN, 0, (50.0, 50.0), None), Step::Pass);
        assert_eq!(step(&mut state, WM_MOUSEMOVE, 0, (312.5, 227.5), Some(7)), Step::Pass);
        assert!(state.inside.is_none());
    }

    #[test]
    fn the_pointer_comes_back_when_the_pad_goes() {
        let mut state = state();
        step(&mut state, WM_MOUSEMOVE, 0, (312.5, 227.5), Some(7));
        state.zones.clear();
        let Step::Move(back) = step(&mut state, WM_MOUSEMOVE, 0, (2561.0, 360.0), None) else { panic!() };
        assert!(near(back, (313, 228)), "{back:?}");
        assert!(state.inside.is_none());
    }

    /// Puts the real hook in and checks it comes out again on its own. Its only
    /// zone belongs to no window, so the mouse is never taken.
    #[test]
    #[ignore = "briefly hooks the real mouse"]
    fn the_hook_goes_in_and_comes_out() {
        set_zones(vec![Zone { window: -1, ..zone() }]);
        std::thread::sleep(Duration::from_millis(300));
        assert!(HOOKED.load(Ordering::SeqCst), "Windows refused the hook");
        set_zones(Vec::new());
        std::thread::sleep(Duration::from_millis(500));
        assert!(!HOOKED.load(Ordering::SeqCst), "the hook stayed in");
    }
}
