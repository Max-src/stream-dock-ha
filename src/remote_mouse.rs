//! The mouse on Mini Monitor from a Home Assistant dashboard card
//! (docs/integration-protocol.md, `mouse`). The card sends where on the Mini
//! Monitor picture it was pressed, as parts of its width and height, and the
//! real Windows pointer goes to that point of the extra display, as with
//! Mouse on Mini Monitor on a pad on screen. Off unless the person turns on
//! "Cards can use the mouse", in the Monitor tab.
//!
//! Everything is sent with `SendInput`, positions included, so it's marked as
//! injected and the pad on screen's mouse hook lets it through.

use crate::model::PadKind;
use crate::monitor::{desktop_point_clamped, Area};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub const MOUSE_OFF: &str = "Mouse from dashboard cards is off in Stream Dock HA (Monitor tab)";
pub const NOT_SHOWING: &str = "Mini Monitor isn't showing on this pad";
/// A button a card pressed and never let go of is let go after this long.
const STUCK: Duration = Duration::from_secs(10);
/// Wheel notches in one go, at most, either way.
const MOST_NOTCHES: i64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Move,
    Down(Button),
    Up(Button),
    Click(Button),
    /// Wheel notches, away from the person (up) when positive.
    Scroll(i32),
}

/// What a `mouse` command asks for: what to do, where on the picture (parts
/// of its width and height, 0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    pub act: Act,
    pub at: (f32, f32),
}

/// Reads a `mouse` command, or says what's wrong with it.
pub fn request_of(data: &Value) -> Result<Request, String> {
    let part = |field: &str| -> Result<f32, String> {
        let value = data.get(field).and_then(Value::as_f64).ok_or(format!("This needs \"{field}\", from 0 to 1"))?;
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(format!("\"{field}\" should be from 0 to 1"));
        }
        Ok(value as f32)
    };
    let at = (part("x")?, part("y")?);
    let button = match data.get("button") {
        None | Some(Value::Null) => Button::Left,
        Some(value) => match value.as_str() {
            Some("left") => Button::Left,
            Some("right") => Button::Right,
            Some("middle") => Button::Middle,
            _ => return Err("\"button\" should be left, right or middle".into()),
        },
    };
    let act = match data.get("action").and_then(Value::as_str) {
        Some("move") => Act::Move,
        Some("down") => Act::Down(button),
        Some("up") => Act::Up(button),
        Some("click") => Act::Click(button),
        Some("scroll") => {
            let notches = match data.get("amount") {
                None | Some(Value::Null) => 1,
                Some(value) => value.as_i64().ok_or("\"amount\" should be a whole number of wheel notches")?,
            };
            Act::Scroll(notches.clamp(-MOST_NOTCHES, MOST_NOTCHES) as i32)
        }
        Some(other) => return Err(format!("\"action\" should be move, down, up, click or scroll, not \"{other}\"")),
        None => return Err("This needs \"action\": move, down, up, click or scroll".into()),
    };
    Ok(Request { act, at })
}

/// The desktop point a point of the picture shows: `size` is the picture's,
/// `from` the display it was taken of.
pub fn desktop_of(at: (f32, f32), size: (i32, i32), from: Area) -> (i32, i32) {
    desktop_point_clamped(from, size, at.0 * size.0 as f32, at.1 * size.1 as f32, 0)
}

/// Does what a card asked on the pad of that kind's Mini Monitor.
pub fn act(kind: PadKind, request: Request) -> Result<(), String> {
    if kind == PadKind::Dashboard {
        return Err("A virtual pad can't show Mini Monitor".into());
    }
    let shown = crate::monitor::shown(kind).and_then(|shown| Some(((shown.layout.width, shown.layout.height), shown.from?)));
    act_on(shown, request)
}

/// Does it on a picture `size` big taken of the display at `from`, if there's one.
fn act_on(shown: Option<((i32, i32), Area)>, request: Request) -> Result<(), String> {
    let (size, from) = shown.ok_or(NOT_SHOWING)?;
    let point = desktop_of(request.at, size, from);
    // Remembers where the pointer was, to give it back (pointer_back).
    crate::pointer_back::card_used(from);
    match request.act {
        Act::Move => send(point, Input::Move),
        Act::Down(button) => {
            send(point, Input::Button(button, true));
            hold(button);
        }
        Act::Up(button) => {
            send(point, Input::Button(button, false));
            let_go(button);
        }
        Act::Click(button) => {
            send(point, Input::Button(button, true));
            send(point, Input::Button(button, false));
        }
        Act::Scroll(notches) => send(point, Input::Wheel(notches)),
    }
    Ok(())
}

/// One thing to send, at a desktop point.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Input {
    Move,
    Button(Button, bool),
    Wheel(i32),
}

/// The buttons a card is holding down, each with the serial of its press.
static HELD: Mutex<[Option<u64>; 3]> = Mutex::new([None; 3]);
static SERIAL: AtomicU64 = AtomicU64::new(0);

fn slot(button: Button) -> usize {
    match button {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

/// Notes a press, and lets the button go by itself if no release comes.
fn hold(button: Button) {
    let serial = SERIAL.fetch_add(1, Ordering::SeqCst) + 1;
    crate::model::lock(&HELD)[slot(button)] = Some(serial);
    std::thread::spawn(move || {
        std::thread::sleep(STUCK);
        let still = {
            let mut held = crate::model::lock(&HELD);
            let still = held[slot(button)] == Some(serial);
            if still {
                held[slot(button)] = None;
            }
            still
        };
        if still {
            crate::pad::note("A dashboard card held a mouse button down too long: let go of it");
            send_here(Input::Button(button, false));
        }
    });
}

fn let_go(button: Button) {
    crate::model::lock(&HELD)[slot(button)] = None;
}

/// Whether a card is holding a mouse button down now.
#[cfg_attr(test, allow(dead_code))]
pub fn holding() -> bool {
    crate::model::lock(&HELD).iter().any(Option::is_some)
}

#[cfg(not(test))]
fn send(point: (i32, i32), input: Input) {
    imp::send(Some(point), input);
}

#[cfg(not(test))]
fn send_here(input: Input) {
    imp::send(None, input);
}

// Tests never move the real pointer or press: they see what would be sent.
#[cfg(test)]
static SENT: Mutex<Vec<(Option<(i32, i32)>, Input)>> = Mutex::new(Vec::new());

#[cfg(test)]
fn send(point: (i32, i32), input: Input) {
    crate::model::lock(&SENT).push((Some(point), input));
}

#[cfg(test)]
fn send_here(input: Input) {
    crate::model::lock(&SENT).push((None, input));
}

#[cfg(not(test))]
mod imp {
    use super::{Button, Input};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    };

    /// Sends one input, first moving to `point` (desktop pixels) if given.
    pub fn send(point: Option<(i32, i32)>, input: Input) {
        let (mut flags, mut data) = match input {
            Input::Move => (0, 0),
            Input::Button(button, down) => (
                match (button, down) {
                    (Button::Left, true) => MOUSEEVENTF_LEFTDOWN,
                    (Button::Left, false) => MOUSEEVENTF_LEFTUP,
                    (Button::Right, true) => MOUSEEVENTF_RIGHTDOWN,
                    (Button::Right, false) => MOUSEEVENTF_RIGHTUP,
                    (Button::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
                    (Button::Middle, false) => MOUSEEVENTF_MIDDLEUP,
                },
                0,
            ),
            Input::Wheel(notches) => (MOUSEEVENTF_WHEEL, notches * 120),
        };
        let (mut dx, mut dy) = (0, 0);
        if let Some(point) = point {
            // Absolute positions are 0 to 65535 across the whole desktop.
            (dx, dy) = crate::monitor::absolute(point, crate::monitor::desktop());
            flags |= MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        } else if flags == 0 {
            return;
        }
        if input == Input::Move {
            data = 0;
        }
        let one = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 },
            },
        };
        unsafe {
            SendInput(1, &one, std::mem::size_of::<INPUT>() as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_command_is_read_with_its_defaults() {
        let click = request_of(&json!({"action": "click", "x": 0.5, "y": 0.25})).unwrap();
        assert_eq!(click, Request { act: Act::Click(Button::Left), at: (0.5, 0.25) });
        let right = request_of(&json!({"action": "down", "x": 0, "y": 1, "button": "right"})).unwrap();
        assert_eq!(right.act, Act::Down(Button::Right));
        assert_eq!(request_of(&json!({"action": "scroll", "x": 0.1, "y": 0.1})).unwrap().act, Act::Scroll(1));
        assert_eq!(request_of(&json!({"action": "scroll", "x": 0.1, "y": 0.1, "amount": -300})).unwrap().act, Act::Scroll(-20));
        assert_eq!(request_of(&json!({"action": "up", "x": 0.1, "y": 0.1, "button": "middle"})).unwrap().act, Act::Up(Button::Middle));
        assert_eq!(request_of(&json!({"action": "move", "x": 0.1, "y": 0.1})).unwrap().act, Act::Move);
    }

    #[test]
    fn a_wrong_command_says_what_is_wrong() {
        assert!(request_of(&json!({"action": "click", "x": 1.5, "y": 0.5})).unwrap_err().contains("\"x\""));
        assert!(request_of(&json!({"action": "click", "y": 0.5})).unwrap_err().contains("\"x\""));
        assert!(request_of(&json!({"action": "click", "x": 0.5, "y": 0.5, "button": "side"})).unwrap_err().contains("button"));
        assert!(request_of(&json!({"action": "drag", "x": 0.5, "y": 0.5})).unwrap_err().contains("drag"));
        assert!(request_of(&json!({"x": 0.5, "y": 0.5})).unwrap_err().contains("action"));
        assert!(request_of(&json!({"action": "scroll", "x": 0.5, "y": 0.5, "amount": 1.5})).is_err());
    }

    #[test]
    fn a_point_of_the_picture_is_that_point_of_the_display() {
        // A 1920 x 1080 display to the right of the main one, in a picture of the same shape.
        let display = Area { x: 2560, y: 0, width: 1920, height: 1080 };
        assert_eq!(desktop_of((0.0, 0.0), (480, 270), display), (2560, 0));
        assert_eq!(desktop_of((0.5, 0.5), (480, 270), display), (2560 + 960, 540));
        assert_eq!(desktop_of((1.0, 1.0), (480, 270), display), (2560 + 1919, 1079));
        // A wider picture has bars at the sides: their points go to the display's edge.
        assert_eq!(desktop_of((0.0, 0.5), (640, 270), display).0, 2560);
    }

    #[test]
    fn a_dashboard_pad_or_no_picture_is_refused() {
        let click = Request { act: Act::Click(Button::Left), at: (0.5, 0.5) };
        assert!(act(PadKind::Dashboard, click).is_err());
        assert_eq!(act_on(None, click).unwrap_err(), NOT_SHOWING);
    }

    #[test]
    fn a_click_moves_then_presses_and_lets_go() {
        let shown = Some(((400, 300), Area { x: 100, y: 50, width: 800, height: 600 }));
        let mut sent = lock_sent();
        sent.clear();
        drop(sent);
        act_on(shown, Request { act: Act::Click(Button::Right), at: (0.5, 0.5) }).unwrap();
        act_on(shown, Request { act: Act::Scroll(-2), at: (0.0, 0.0) }).unwrap();
        let sent: Vec<_> = lock_sent().iter().filter(|(point, _)| point.is_some()).cloned().collect();
        assert_eq!(
            sent,
            vec![
                (Some((500, 350)), Input::Button(Button::Right, true)),
                (Some((500, 350)), Input::Button(Button::Right, false)),
                (Some((100, 50)), Input::Wheel(-2)),
            ]
        );
    }

    #[test]
    fn a_release_forgets_the_held_button() {
        hold(Button::Middle);
        assert!(crate::model::lock(&HELD)[slot(Button::Middle)].is_some());
        let_go(Button::Middle);
        assert!(crate::model::lock(&HELD)[slot(Button::Middle)].is_none());
    }

    fn lock_sent() -> std::sync::MutexGuard<'static, Vec<(Option<(i32, i32)>, Input)>> {
        crate::model::lock(&SENT)
    }
}
