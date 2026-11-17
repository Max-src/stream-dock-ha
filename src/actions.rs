//! What a key does: opening things, pressing keys, typing, media keys.

use crate::model::{lock, Action, MediaKey, PadKind, Shared};
use crate::tray;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::System::Shutdown::LockWorkStation;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, IsIconic,
    IsWindowVisible, SetForegroundWindow, GWL_EXSTYLE, GW_HWNDNEXT, SW_SHOWNORMAL,
    WS_EX_TOOLWINDOW,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub vk: u16,
}

fn vk_from_name(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        let c = lower.as_bytes()[0];
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase() as u16);
        }
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        if (1..=24).contains(&n) {
            return Some(VK_F1 + n - 1);
        }
    }
    Some(match lower.as_str() {
        "enter" | "return" => VK_RETURN,
        "tab" => VK_TAB,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "esc" | "escape" => VK_ESCAPE,
        "delete" | "del" => VK_DELETE,
        "insert" | "ins" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "pgup" => VK_PRIOR,
        "pagedown" | "pgdn" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "printscreen" | "prtsc" => VK_SNAPSHOT,
        "pause" => VK_PAUSE,
        "minus" | "-" => VK_OEM_MINUS,
        "plus" | "equals" | "=" => VK_OEM_PLUS,
        "comma" | "," => VK_OEM_COMMA,
        "period" | "." => VK_OEM_PERIOD,
        "semicolon" | ";" => VK_OEM_1,
        "slash" | "/" => VK_OEM_2,
        "backtick" | "`" => VK_OEM_3,
        "openbracket" | "[" => VK_OEM_4,
        "backslash" | "\\" => VK_OEM_5,
        "closebracket" | "]" => VK_OEM_6,
        "quote" | "'" => VK_OEM_7,
        other => {
            // Number pad keys, and any other key by its Windows number.
            if let Some(n) = other.strip_prefix("num").and_then(|n| n.parse::<u16>().ok()) {
                return (n <= 9).then_some(VK_NUMPAD0 + n);
            }
            return other.strip_prefix("key").and_then(|n| n.parse().ok());
        }
    })
}

/// The name a key is saved under. Every name here reads back as the same key.
pub fn key_name(vk: u16) -> String {
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return (vk as u8 as char).to_string();
    }
    if (VK_F1..=VK_F24).contains(&vk) {
        return format!("F{}", vk - VK_F1 + 1);
    }
    if (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk) {
        return format!("Num{}", vk - VK_NUMPAD0);
    }
    let name = match vk {
        VK_RETURN => "Enter",
        VK_TAB => "Tab",
        VK_SPACE => "Space",
        VK_BACK => "Backspace",
        VK_ESCAPE => "Escape",
        VK_DELETE => "Delete",
        VK_INSERT => "Insert",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_UP => "Up",
        VK_DOWN => "Down",
        VK_LEFT => "Left",
        VK_RIGHT => "Right",
        VK_SNAPSHOT => "PrintScreen",
        VK_PAUSE => "Pause",
        VK_OEM_MINUS => "Minus",
        VK_OEM_PLUS => "Plus",
        VK_OEM_COMMA => "Comma",
        VK_OEM_PERIOD => "Period",
        VK_OEM_1 => "Semicolon",
        VK_OEM_2 => "Slash",
        VK_OEM_3 => "Backtick",
        VK_OEM_4 => "OpenBracket",
        VK_OEM_5 => "Backslash",
        VK_OEM_6 => "CloseBracket",
        VK_OEM_7 => "Quote",
        other => return format!("Key{other}"),
    };
    name.to_string()
}

/// Reads text such as "Ctrl+Shift+M".
pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let mut combo = Combo::default();
    for token in text.split('+').map(str::trim).filter(|t| !t.is_empty()) {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "ctl" | "controlleft" | "controlright" => combo.ctrl = true,
            "alt" | "altleft" | "altright" => combo.alt = true,
            "shift" | "shiftleft" | "shiftright" => combo.shift = true,
            "win" | "windows" | "super" | "superleft" | "superright" => combo.win = true,
            _ => {
                if combo.vk != 0 {
                    return Err(format!("\"{text}\" has more than one main key"));
                }
                combo.vk =
                    vk_from_name(token).ok_or_else(|| format!("\"{token}\" isn't a key I know"))?;
            }
        }
    }
    if combo.vk == 0 {
        // One modifier on its own is a key press in its own right.
        let single = match (combo.ctrl, combo.alt, combo.shift, combo.win) {
            (true, false, false, false) => VK_CONTROL,
            (false, true, false, false) => VK_MENU,
            (false, false, true, false) => VK_SHIFT,
            (false, false, false, true) => VK_LWIN,
            _ => return Err(format!("\"{text}\" needs a main key, such as Ctrl+K")),
        };
        return Ok(Combo { vk: single, ..Combo::default() });
    }
    Ok(combo)
}

/// The physical key behind a virtual one. Web pages such as KVM viewers go by
/// the physical key, so a simulated key without it looks like no key at all.
fn scan_code(vk: u16) -> u32 {
    unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) }
}

fn key_input(vk: u16, up: bool) -> INPUT {
    let scan = scan_code(vk);
    let extended = scan >> 8 == 0xE0
        || matches!(
            vk,
            VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_INSERT
                | VK_DELETE | VK_LWIN | VK_RWIN | VK_PAUSE | VK_SNAPSHOT | VK_NUMLOCK | VK_SEPARATOR
                | VK_DIVIDE | VK_RCONTROL | VK_RMENU
        );
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan as u16 & 0xFF,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("Windows blocked the key press (a program running as administrator may be in front)".into())
    }
}

/// How long a key is held before letting go, on every way of sending one.
/// A key that's barely
/// down can be missed by anything between here and the program, such as a KVM.
const KEY_HOLD_MS: u64 = 60;
/// The gap between a key's steps.
const STEP_GAP_MS: u64 = 100;
/// How long each typed key stays down. It's short, or long text would take
/// minutes, but enough for a remote link to see two of the same key in a row.
const TYPED_KEY_MS: u64 = 10;

/// Win+L can't be simulated: Windows only takes it from a real keyboard. The
/// original app has the same rule, with or without other keys.
fn locks_the_pc(combo: Combo) -> bool {
    combo.win && combo.vk == 0x4C
}

fn lock_the_pc() -> Result<(), String> {
    if unsafe { LockWorkStation() } != 0 {
        Ok(())
    } else {
        Err("Windows didn't lock the PC".into())
    }
}

/// Presses one key and lets go, as the original does with media keys.
fn tap(vk: u16) -> Result<(), String> {
    send(&[key_input(vk, false)])?;
    std::thread::sleep(Duration::from_millis(KEY_HOLD_MS));
    send(&[key_input(vk, true)])
}

fn press_combo(combo: Combo) -> Result<(), String> {
    if locks_the_pc(combo) {
        return lock_the_pc();
    }
    let mut mods = Vec::new();
    if combo.ctrl {
        mods.push(VK_CONTROL);
    }
    if combo.alt {
        mods.push(VK_MENU);
    }
    if combo.shift {
        mods.push(VK_SHIFT);
    }
    if combo.win {
        mods.push(VK_LWIN);
    }
    let mut inputs: Vec<INPUT> = mods.iter().map(|&m| key_input(m, false)).collect();
    inputs.push(key_input(combo.vk, false));
    send(&inputs)?;
    std::thread::sleep(Duration::from_millis(KEY_HOLD_MS));
    let mut release = vec![key_input(combo.vk, true)];
    release.extend(mods.iter().rev().map(|&m| key_input(m, true)));
    send(&release)
}

/// The text to type, with Enter at the end if it's wanted. A line break is
/// typed as Enter.
fn with_enter(text: &str, press_enter: bool) -> String {
    if press_enter {
        format!("{text}\n")
    } else {
        text.to_string()
    }
}

/// How to type a character with real keys on this keyboard layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RealKey {
    vk: u16,
    shift: bool,
    /// Ctrl and Alt together, which is what the AltGr key sends.
    alt_gr: bool,
}

/// The real key for a character, if it has one. Dead keys don't count, because
/// they only type something together with the next key.
fn real_key(unit: u16, caps_lock: bool) -> Option<RealKey> {
    let found = unsafe { VkKeyScanW(unit) };
    if found == -1 {
        return None;
    }
    let (vk, state) = (found as u16 & 0xFF, (found as u16 >> 8) & 0xFF);
    let dead = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) } & 0x8000_0000 != 0;
    if dead || state & !7 != 0 || (state & 6 != 0 && state & 6 != 6) {
        return None;
    }
    let letter = (0x41..=0x5A).contains(&vk);
    Some(RealKey { vk, shift: (state & 1 != 0) != (caps_lock && letter), alt_gr: state & 6 == 6 })
}

/// The keys to go down, then the keys to come up, for one typed character.
fn real_key_presses(key: RealKey) -> (Vec<INPUT>, Vec<INPUT>) {
    let mut held = Vec::new();
    if key.alt_gr {
        held.extend([VK_CONTROL, VK_MENU]);
    }
    if key.shift {
        held.push(VK_SHIFT);
    }
    let mut down: Vec<INPUT> = held.iter().map(|vk| key_input(*vk, false)).collect();
    down.push(key_input(key.vk, false));
    let mut up = vec![key_input(key.vk, true)];
    up.extend(held.iter().rev().map(|vk| key_input(*vk, true)));
    (down, up)
}

/// Real keys wherever the keyboard has them, so text reaches anything that
/// reads keys, such as a KVM viewer. Other characters go as plain text.
fn type_text(text: &str) -> Result<(), String> {
    let caps_lock = unsafe { GetKeyState(VK_CAPITAL as i32) } & 1 != 0;
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        if unit == b'\r' as u16 {
            continue;
        }
        let real = if unit == b'\n' as u16 {
            Some(RealKey { vk: VK_RETURN, shift: false, alt_gr: false })
        } else {
            real_key(unit, caps_lock)
        };
        if let Some(key) = real {
            let (down, up) = real_key_presses(key);
            send(&down)?;
            std::thread::sleep(Duration::from_millis(TYPED_KEY_MS));
            send(&up)?;
            continue;
        }
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: unit,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
        send(&inputs)?;
        inputs.clear();
    }
    Ok(())
}

fn shell_open(target: &str, args: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("No program or file chosen for this key yet".into());
    }
    let (verb, file, params) = (crate::win::wide("open"), crate::win::wide(target), crate::win::wide(args));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            if args.is_empty() {
                std::ptr::null()
            } else {
                params.as_ptr()
            },
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize > 32 {
        Ok(())
    } else {
        Err(format!("Windows couldn't open \"{target}\""))
    }
}

pub fn website_address(url: &str) -> String {
    let url = url.trim();
    if url.contains("://") || url.starts_with("mailto:") {
        url.to_string()
    } else {
        format!("https://{url}")
    }
}

/// The keyboard's media key for it. The microphone has none, so it always
/// goes through Windows' audio settings, as the original's does.
fn media_vk(key: MediaKey) -> Option<u16> {
    Some(match key {
        MediaKey::VolumeUp => VK_VOLUME_UP,
        MediaKey::VolumeDown => VK_VOLUME_DOWN,
        MediaKey::Mute => VK_VOLUME_MUTE,
        MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
        MediaKey::NextTrack => VK_MEDIA_NEXT_TRACK,
        MediaKey::PreviousTrack => VK_MEDIA_PREV_TRACK,
        MediaKey::Stop => VK_MEDIA_STOP,
        MediaKey::MicMute | MediaKey::MicUp | MediaKey::MicDown => return None,
    })
}

fn run_one(shared: &Shared, profile: Option<usize>, action: &Action) -> Result<(), String> {
    match action {
        Action::Open { target, args } => shell_open(target, args),
        Action::Website { url } => {
            if url.trim().is_empty() {
                return Err("No website address for this key yet".into());
            }
            shell_open(&website_address(url), "")
        }
        Action::Shortcut { keys } => press_combo(parse_combo(keys)?),
        Action::TypeText { text, press_enter } => type_text(&with_enter(text, *press_enter)),
        Action::Media { key } => {
            let Some(vk) = media_vk(*key) else {
                crate::media::press_without_keyboard(*key)?;
                // Keys showing whether the microphone is muted change at once.
                crate::pad::wake();
                shared.touch();
                return Ok(());
            };
            // Windows refuses simulated keys while the screen is locked.
            tap(vk).or_else(|_| crate::media::press_without_keyboard(*key))
        }
        Action::Wait { ms } => {
            std::thread::sleep(Duration::from_millis((*ms).min(60_000) as u64));
            Ok(())
        }
        Action::NextPage | Action::PreviousPage | Action::GoToPage { .. } => {
            let mut cfg = lock(&shared.cfg);
            let index = profile.unwrap_or_else(|| cfg.main_index());
            turn_page(&mut cfg, index, action);
            drop(cfg);
            crate::pad::wake();
            shared.touch();
            Ok(())
        }
        Action::HomeAssistant(binding) => shared.ha.call(&binding.entity, &binding.service, &binding.data),
        Action::ToggleMiniMonitor => {
            {
                let mut cfg = lock(&shared.cfg);
                let kind = profile.and_then(|i| cfg.profiles.get(i)).map_or(PadKind::Main, |p| p.pad);
                if !kind.has_mini_monitor() {
                    return Err("A virtual pad can't show Mini Monitor".into());
                }
                if !cfg.mini_monitor {
                    cfg.monitor_pad = kind;
                }
                cfg.mini_monitor = !cfg.mini_monitor;
            }
            shared.save_now();
            crate::pad::wake();
            shared.touch();
            Ok(())
        }
        Action::AdjustBrightness { change } => {
            {
                let mut cfg = lock(&shared.cfg);
                let pad_id = pad_of_key(&cfg, profile);
                cfg.adjust_pad_brightness(&pad_id, *change);
            }
            shared.save_now();
            crate::pad::wake();
            shared.touch();
            Ok(())
        }
        Action::NextProfile | Action::PreviousProfile | Action::FirstProfile => {
            let mut cfg = lock(&shared.cfg);
            let pad_id = pad_of_key(&cfg, profile);
            let target = match action {
                Action::NextProfile => cfg.profile_along_pad(&pad_id, 1),
                Action::PreviousProfile => cfg.profile_along_pad(&pad_id, -1),
                _ => cfg.profiles_of(&pad_id).first().copied(),
            };
            if let Some(target) = target {
                cfg.run_profile(target);
            }
            drop(cfg);
            tray::reload();
            crate::pad::wake();
            shared.touch();
            Ok(())
        }
        Action::SwitchProfile { name } => {
            let mut cfg = lock(&shared.cfg);
            let Some(i) = cfg.profiles.iter().position(|p| &p.name == name) else {
                return Err(format!("There's no profile called \"{name}\""));
            };
            cfg.run_profile(i);
            drop(cfg);
            tray::reload();
            crate::pad::wake();
            shared.touch();
            Ok(())
        }
    }
}

/// The pad a key's steps act on: the pad of its profile, or the 293S for a
/// key run from a shortcut.
fn pad_of_key(cfg: &crate::model::Config, profile: Option<usize>) -> String {
    let pad = match profile {
        Some(index) => cfg.pad_of_profile(index),
        None => cfg.pad_of_kind(PadKind::Main),
    };
    pad.map_or(String::new(), |entry| entry.id.clone())
}

/// Turns the page of one profile, so each pad keeps its own.
fn turn_page(cfg: &mut crate::model::Config, profile: usize, action: &Action) {
    let Some(p) = cfg.profiles.get_mut(profile) else { return };
    let count = p.pages.len();
    p.active_page = match action {
        Action::NextPage => (p.active_page + 1) % count,
        Action::PreviousPage => (p.active_page + count - 1) % count,
        Action::GoToPage { page } => page.saturating_sub(1).min(count - 1),
        _ => p.active_page,
    };
}

/// With a shortcut like Ctrl+Alt+1 the person's fingers are still on Ctrl and
/// Alt, which would mix with any keys the action presses.
fn wait_for_modifiers_released() {
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        let held = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|&vk| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0);
        if !held {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Pressing an on-screen key puts this window in front, but "Paste" or "Type
/// text" are meant for the program the person was using, so that program gets
/// the keyboard back first.
fn give_keyboard_back(shared: &Shared) {
    let own = shared.window.load(Ordering::SeqCst) as HWND;
    unsafe {
        if own.is_null() || GetForegroundWindow() != own {
            return;
        }
        let mut next = GetWindow(own, GW_HWNDNEXT);
        while !next.is_null() {
            let mut cloaked = 0u32;
            DwmGetWindowAttribute(
                next,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
            );
            let tool_window = GetWindowLongPtrW(next, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0;
            if IsWindowVisible(next) != 0
                && IsIconic(next) == 0
                && GetWindowTextLengthW(next) > 0
                && cloaked == 0
                && !tool_window
            {
                SetForegroundWindow(next);
                std::thread::sleep(Duration::from_millis(150));
                return;
            }
            next = GetWindow(next, GW_HWNDNEXT);
        }
    }
}

/// Where a press came from. Home Assistant hears it with each press, and it
/// decides what a press may do: Home Assistant's own presses ask the key
/// first (`remote::may_press`), and only presses made while looking at the
/// pad bring its keys back from Mini Monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PressSource {
    /// The physical pad.
    Pad,
    /// A pad on screen in the app.
    Screen,
    /// A Home Assistant dashboard card.
    Dashboard,
    /// A Home Assistant action or event command.
    Action,
    /// A keyboard shortcut.
    Hotkey,
}

impl PressSource {
    /// Whether the person is pressing it themselves, on this PC.
    pub fn is_local(self) -> bool {
        matches!(self, PressSource::Pad | PressSource::Screen | PressSource::Hotkey)
    }

    /// Whether whoever pressed is looking at the pad itself, which can't
    /// show its keys under Mini Monitor.
    fn looks_at_the_pad(self) -> bool {
        matches!(self, PressSource::Pad | PressSource::Screen)
    }
}

/// What pressing a control does, whether on the pad itself or on its picture
/// on screen: the key covering that cell on the pad's running profile and
/// page runs, Home Assistant hears about it, and the window lights it up.
pub fn press_control(shared: &Arc<Shared>, kind: PadKind, cell: usize, source: PressSource) {
    let pad_id = lock(&shared.cfg).pad_of_kind(kind).map(|entry| entry.id.clone());
    if let Some(pad_id) = pad_id {
        press_pad(shared, &pad_id, cell, source);
    }
}

/// `press_control` for a pad by id: how a pad on screen is pressed, dashboard
/// pads included.
pub fn press_pad(shared: &Arc<Shared>, pad_id: &str, cell: usize, source: PressSource) {
    if let Err(problem) = press(shared, pad_id, cell, None, source) {
        crate::pad::note(&format!("press refused: {problem}"));
    }
}

/// The press itself, for any source. `at` names a profile and page of the
/// pad's, for a command that presses a key not showing; otherwise it's the
/// running profile at its current page. It says whether the press only
/// dismissed something (an alert, a dark screen, Mini Monitor), or why it
/// wasn't allowed.
pub fn press(shared: &Arc<Shared>, pad_id: &str, cell: usize, at: Option<(usize, usize)>, source: PressSource) -> Result<bool, String> {
    let Some((kind, label)) = lock(&shared.cfg).pad(pad_id).map(|entry| (entry.kind, entry.name.clone())) else {
        return Err("That pad isn't there any more".into());
    };
    // An alert covers the keys, so pressing anything on the pad only takes it
    // away. A shortcut isn't a press on the pad: it runs and leaves the alert up.
    if source != PressSource::Hotkey && crate::alerts::clear(shared, pad_id) {
        crate::pad::note(&format!("{label}: press, alert dismissed"));
        dismissed(shared, pad_id, cell, source);
        return Ok(true);
    }
    // A shortcut doesn't need the pad's screen, so it neither wakes it nor waits for it.
    if source != PressSource::Hotkey {
        let was_asleep = crate::screen_off::pad_asleep(&lock(&shared.cfg), pad_id);
        crate::screen_off::wake_pad(pad_id, kind);
        if was_asleep {
            crate::pad::note(&format!("{label}: press, waking the screen"));
            crate::pad::wake();
            dismissed(shared, pad_id, cell, source);
            return Ok(true);
        }
    }
    // While a pad shows the alignment picture or Mini Monitor nobody can see
    // its keys, so a press on it only means "back to my keys", as on the real
    // pad. A card or an automation can see the keys, so its press runs them.
    if source.looks_at_the_pad() && back_to_the_keys(shared, kind, &label) {
        dismissed(shared, pad_id, cell, source);
        return Ok(true);
    }
    // Something Home Assistant put on this key: the press takes it away, as
    // for an alert, but only on the page the pad is showing.
    let running = {
        let cfg = lock(&shared.cfg);
        cfg.pad_profile_of(pad_id).map(|index| (index, cfg.profiles[index].active_page))
    };
    if source != PressSource::Hotkey && (at.is_none() || at == running) && crate::flashes::clear_key(shared, pad_id, cell) {
        crate::pad::note(&format!("{label}: press, flash on cell {cell} dismissed"));
        dismissed(shared, pad_id, cell, source);
        return Ok(true);
    }
    let cfg = lock(&shared.cfg);
    let Some((index, page)) = at.or_else(|| cfg.pad_profile_of(pad_id).map(|index| (index, cfg.profiles[index].active_page))) else {
        return Err(format!("The {label} pad isn't in use yet"));
    };
    let profile = &cfg.profiles[index];
    let slot = profile.owner_of(page, cell);
    let Some(key) = profile.pages.get(page).and_then(|page| page.keys.get(slot)) else {
        return Err("That pad has no such key".into());
    };
    crate::remote::may_press(&cfg, key, source)?;
    let asks_twice = crate::confirm::needs(key);
    let (name, steps) = (key.title.replace('\n', " "), key.actions.clone());
    let press = crate::bridge::Press::of(&cfg, pad_id, index, page, slot, source, false);
    drop(cfg);
    // A key that asks for a second press: the first only says "Press again".
    if asks_twice && !crate::confirm::press(shared, pad_id, index, page, slot) {
        crate::pad::note(&format!("{label}: cell {slot} pressed, waiting for a second press ({source:?})"));
        if source != PressSource::Hotkey {
            mark_pressed(shared, kind, pad_id, slot);
        }
        return Ok(false);
    }
    crate::pad::note(&format!("{label}: cell {slot} pressed, {name} ({source:?})"));
    *lock(&shared.last_press) = Some(std::time::SystemTime::now());
    if source != PressSource::Hotkey {
        mark_pressed(shared, kind, pad_id, slot);
    }
    // The older press events keep what they always heard: presses on a pad,
    // on screen or now on a card, but not Home Assistant's own or shortcuts.
    if matches!(source, PressSource::Pad | PressSource::Screen | PressSource::Dashboard) {
        crate::remote::announce(shared, pad_id, slot);
    }
    if let Some(press) = press {
        crate::bridge::pressed(shared, press);
    }
    shared.touch();
    if !steps.is_empty() {
        run_key(shared.clone(), Some(index), name, steps, true);
    }
    Ok(false)
}

/// Takes a pad out of Mini Monitor or the alignment picture, if it's showing
/// one, and says whether it was.
fn back_to_the_keys(shared: &Shared, kind: PadKind, label: &str) -> bool {
    let mut cfg = lock(&shared.cfg);
    let pattern = cfg.monitor_pad == kind && kind.has_mini_monitor() && shared.alignment_pattern.swap(false, Ordering::SeqCst);
    let monitor = cfg.monitor_on(kind);
    if !pattern && !monitor {
        return false;
    }
    cfg.mini_monitor &= !monitor;
    drop(cfg);
    crate::pad::note(&format!("{label}: press, back to the keys"));
    if monitor {
        shared.save_now();
    }
    crate::pad::wake();
    shared.touch();
    true
}

/// A press that only took something away: it's a press all the same, so it
/// counts as the last one, and the integration hears it was only that.
fn dismissed(shared: &Shared, pad_id: &str, cell: usize, source: PressSource) {
    *lock(&shared.last_press) = Some(std::time::SystemTime::now());
    let press = {
        let cfg = lock(&shared.cfg);
        cfg.pad_profile_of(pad_id).and_then(|index| {
            let page = cfg.profiles[index].active_page;
            let slot = cfg.profiles[index].owner_of(page, cell);
            crate::bridge::Press::of(&cfg, pad_id, index, page, slot, source, true)
        })
    };
    if let Some(press) = press {
        crate::bridge::pressed(shared, press);
    }
    shared.touch();
}

/// Lights up a pressed control in the window and on screen for a moment.
pub fn mark_pressed(shared: &Shared, kind: PadKind, pad_id: &str, slot: usize) {
    match kind {
        PadKind::Main => *lock(&shared.pad_press) = Some((slot, Instant::now())),
        PadKind::Akp03e => crate::small_pad::mark_pressed(slot),
        PadKind::Dashboard => *lock(&shared.dashboard_press) = Some((pad_id.to_string(), slot, Instant::now())),
    }
}

/// Whether a control was pressed just now, to light it up.
pub fn just_pressed(shared: &Shared, kind: PadKind, pad_id: &str, slot: usize) -> bool {
    let recent = |at: Instant| at.elapsed().as_millis() < 300;
    match kind {
        PadKind::Main => lock(&shared.pad_press).is_some_and(|(cell, at)| cell == slot && recent(at)),
        PadKind::Akp03e => crate::small_pad::just_pressed(slot),
        PadKind::Dashboard => lock(&shared.dashboard_press).as_ref().is_some_and(|(id, cell, at)| id == pad_id && *cell == slot && recent(*at)),
    }
}

/// Runs a key's steps in order, away from the window and the tray so a long
/// "Wait" step holds nothing up. `profile` is the profile the key belongs to,
/// for its page actions; without one they turn the main pad's page.
pub fn run_key(shared: Arc<Shared>, profile: Option<usize>, name: String, actions: Vec<Action>, from_hotkey: bool) {
    if actions.is_empty() {
        shared.set_status(format!("\"{name}\" has nothing to do yet"));
        return;
    }
    std::thread::spawn(move || {
        unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        }
        if from_hotkey {
            wait_for_modifiers_released();
        } else if actions
            .iter()
            .any(|a| matches!(a, Action::Shortcut { .. } | Action::TypeText { .. }))
        {
            give_keyboard_back(&shared);
        }
        for (step, action) in actions.iter().enumerate() {
            if step > 0 {
                std::thread::sleep(Duration::from_millis(STEP_GAP_MS));
            }
            if let Err(message) = run_one(&shared, profile, action) {
                crate::pad::note(&format!("key \"{name}\" failed: {message}"));
                shared.set_status(message);
                return;
            }
        }
        shared.set_status(format!("Ran \"{name}\""));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_mini_monitor_only_the_pad_brings_the_keys_back() {
        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-actions-test.json"));
        let mut cfg = crate::model::Config::default();
        cfg.ha_control = true;
        cfg.mini_monitor = true;
        cfg.monitor_pad = PadKind::Main;
        let main = cfg.pads[0].id.clone();
        let (shared, _opened) = Shared::new(cfg, true);
        // A card or an automation runs the key and leaves Mini Monitor on.
        assert_eq!(press(&shared, &main, 0, None, PressSource::Dashboard), Ok(false));
        assert_eq!(press(&shared, &main, 0, None, PressSource::Action), Ok(false));
        assert!(lock(&shared.cfg).mini_monitor);
        // A press on the pad only brings the keys back.
        assert_eq!(press(&shared, &main, 0, None, PressSource::Pad), Ok(true));
        assert!(!lock(&shared.cfg).mini_monitor);
        assert_eq!(press(&shared, &main, 0, None, PressSource::Pad), Ok(false));
        assert!(lock(&shared.last_press).is_some());
    }

    #[test]
    fn an_alert_takes_a_press_on_the_pad_from_any_source_but_not_a_shortcut() {
        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-actions-test.json"));
        let mut cfg = crate::model::Config::default();
        cfg.profiles[0].pages[0].keys[3] = crate::model::DockKey { title: "Nothing".into(), actions: Vec::new(), ..Default::default() };
        let main = cfg.pads[0].id.clone();
        let (shared, _opened) = Shared::new(cfg, true);
        for source in [PressSource::Pad, PressSource::Screen, PressSource::Dashboard, PressSource::Action] {
            crate::alerts::show(&shared, &main, crate::alerts::from_command(&serde_json::json!({"title": "Hi"})).unwrap());
            assert_eq!(press(&shared, &main, 3, None, source), Ok(true), "{source:?}");
            assert!(crate::alerts::active(&shared, &main).is_none());
        }
        // A shortcut runs its key as usual, and the alert stays up.
        crate::alerts::show(&shared, &main, crate::alerts::from_command(&serde_json::json!({"title": "Hi"})).unwrap());
        assert_eq!(press(&shared, &main, 3, None, PressSource::Hotkey), Ok(false));
        assert!(crate::alerts::active(&shared, &main).is_some());
        assert!(lock(&shared.last_press).is_some(), "it ran");
        crate::alerts::clear(&shared, &main);
    }

    /// Pages wrap both ways, as in the original app.
    #[test]
    fn pages_wrap_round_at_both_ends_like_the_original() {
        let mut cfg = crate::model::Config::default();
        cfg.profiles[0].add_page();
        cfg.profiles[0].add_page();
        cfg.profiles[0].active_page = 2;
        turn_page(&mut cfg, 0, &Action::NextPage);
        assert_eq!(cfg.profiles[0].active_page, 0);
        turn_page(&mut cfg, 0, &Action::PreviousPage);
        assert_eq!(cfg.profiles[0].active_page, 2);
    }

    /// What Windows really hands to programs for a simulated key. F24 is a key
    /// nothing reacts to.
    #[test]
    #[ignore = "sends a key to this PC"]
    fn windows_passes_the_physical_key_on() {
        use std::sync::atomic::AtomicU32;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT,
            MSG, PM_REMOVE, WH_KEYBOARD_LL,
        };
        static SEEN: AtomicU32 = AtomicU32::new(u32::MAX);
        unsafe extern "system" fn hook(code: i32, wparam: usize, lparam: isize) -> isize {
            let key = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            if code >= 0 && key.vkCode == VK_F24 as u32 {
                SEEN.store(key.scanCode, Ordering::SeqCst);
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
        }
        let pump = |wanted: u32| unsafe {
            let mut message: MSG = std::mem::zeroed();
            let started = Instant::now();
            while SEEN.load(Ordering::SeqCst) == wanted && started.elapsed() < Duration::from_secs(2) {
                while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    DispatchMessageW(&message);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        unsafe {
            let hooked = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), std::ptr::null_mut(), 0);
            assert!(!hooked.is_null());
            let mut old_way = key_input(VK_F24, false);
            old_way.Anonymous.ki.wScan = 0;
            let mut old_up = key_input(VK_F24, true);
            old_up.Anonymous.ki.wScan = 0;
            send(&[old_way, old_up]).unwrap();
            pump(u32::MAX);
            println!("without a scan code, programs see {}", SEEN.load(Ordering::SeqCst));
            let before = SEEN.load(Ordering::SeqCst);
            send(&[key_input(VK_F24, false), key_input(VK_F24, true)]).unwrap();
            pump(before);
            println!("now programs see {}", SEEN.load(Ordering::SeqCst));
            UnhookWindowsHookEx(hooked);
            assert_ne!(SEEN.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn simulated_keys_carry_their_physical_key() {
        for vk in [0x41, VK_RETURN, VK_SHIFT, VK_CONTROL, VK_MENU, VK_TAB, VK_F5] {
            let scan = unsafe { key_input(vk, false).Anonymous.ki.wScan };
            assert_ne!(scan, 0, "key {vk}");
        }
    }

    #[test]
    fn extended_keys_are_marked_as_extended() {
        for vk in [VK_RIGHT, VK_DELETE, VK_LWIN] {
            let flags = unsafe { key_input(vk, true).Anonymous.ki.dwFlags };
            assert_ne!(flags & KEYEVENTF_EXTENDEDKEY, 0, "key {vk}");
        }
        assert_eq!(unsafe { key_input(VK_RETURN, false).Anonymous.ki.dwFlags } & KEYEVENTF_EXTENDEDKEY, 0);
    }

    #[test]
    fn letters_are_typed_with_real_keys() {
        let lower = real_key('a' as u16, false).unwrap();
        assert_eq!((lower.vk, lower.shift, lower.alt_gr), (0x41, false, false));
        assert!(real_key('A' as u16, false).unwrap().shift);
    }

    #[test]
    fn caps_lock_swaps_shift_for_letters_only() {
        assert!(real_key('a' as u16, true).unwrap().shift);
        assert!(!real_key('A' as u16, true).unwrap().shift);
        let digit = real_key(' ' as u16, true).unwrap();
        assert!(!digit.shift);
    }

    #[test]
    fn characters_with_no_key_fall_back_to_plain_text() {
        let emoji: Vec<u16> = "😀".encode_utf16().collect();
        assert!(emoji.iter().all(|unit| real_key(*unit, false).is_none()));
    }

    #[test]
    fn modifiers_wrap_the_key_and_let_go_in_reverse() {
        let (down, up) = real_key_presses(RealKey { vk: 0x41, shift: true, alt_gr: false });
        let steps: Vec<(u16, bool)> = down
            .iter()
            .chain(&up)
            .map(|input| unsafe { (input.Anonymous.ki.wVk, input.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP != 0) })
            .collect();
        assert_eq!(steps, [(VK_SHIFT, false), (0x41, false), (0x41, true), (VK_SHIFT, true)]);
    }

    #[test]
    fn win_l_locks_the_pc_like_the_original() {
        assert!(locks_the_pc(parse_combo("Win+L").unwrap()));
        assert!(locks_the_pc(parse_combo("Ctrl+Win+L").unwrap()));
        assert!(!locks_the_pc(parse_combo("Win+D").unwrap()));
        assert!(!locks_the_pc(parse_combo("Ctrl+L").unwrap()));
    }

    #[test]
    fn every_key_the_original_calls_extended_is_marked() {
        let keys = [
            VK_PAUSE, VK_PRIOR, VK_NEXT, VK_END, VK_HOME, VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN, VK_SNAPSHOT, VK_INSERT,
            VK_DELETE, VK_SEPARATOR, VK_DIVIDE, VK_NUMLOCK, VK_RCONTROL, VK_RMENU,
        ];
        for vk in keys {
            let flags = unsafe { key_input(vk, false).Anonymous.ki.dwFlags };
            assert_ne!(flags & KEYEVENTF_EXTENDEDKEY, 0, "key {vk}");
        }
    }

    #[test]
    fn enter_is_only_added_when_asked_for() {
        assert_eq!(with_enter("hello", false), "hello");
        assert_eq!(with_enter("hello", true), "hello\n");
        assert_eq!(with_enter("", true), "\n");
    }

    #[test]
    fn each_pad_keeps_its_own_page() {
        let mut cfg = crate::model::Config::default();
        cfg.profiles[0].add_page();
        cfg.add_small_pad("Small");
        cfg.profiles[1].add_page();
        cfg.profiles[0].active_page = 0;
        cfg.profiles[1].active_page = 0;
        turn_page(&mut cfg, 1, &Action::NextPage);
        assert_eq!((cfg.profiles[0].active_page, cfg.profiles[1].active_page), (0, 1));
        turn_page(&mut cfg, 0, &Action::NextPage);
        assert_eq!((cfg.profiles[0].active_page, cfg.profiles[1].active_page), (1, 1));
    }

    #[test]
    fn reads_shortcuts() {
        let c = parse_combo("Ctrl+Shift+M").unwrap();
        assert!(c.ctrl && c.shift && !c.alt && !c.win);
        assert_eq!(c.vk, b'M' as u16);
        assert_eq!(parse_combo("win + shift + s").unwrap().vk, b'S' as u16);
        assert_eq!(parse_combo("F13").unwrap().vk, VK_F1 + 12);
        assert_eq!(parse_combo("Alt+Down").unwrap().vk, VK_DOWN);
    }

    #[test]
    fn reads_names_the_old_recorder_saved() {
        assert_eq!(parse_combo("SuperLeft").unwrap().vk, VK_LWIN);
        let c = parse_combo("SuperLeft+S").unwrap();
        assert!(c.win);
        assert_eq!(c.vk, b'S' as u16);
        assert_eq!(parse_combo("ControlLeft").unwrap().vk, VK_CONTROL);
    }

    #[test]
    fn every_key_name_reads_back_as_the_same_key() {
        for vk in 1..=254u16 {
            let modifier = matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
            if modifier {
                continue;
            }
            let name = key_name(vk);
            assert_eq!(parse_combo(&name).unwrap().vk, vk, "{name}");
            assert_eq!(parse_combo(&format!("Win+Shift+{name}")).unwrap().vk, vk, "{name}");
        }
    }

    #[test]
    fn refuses_bad_shortcuts() {
        assert!(parse_combo("").is_err());
        assert!(parse_combo("Ctrl+Alt").is_err());
        assert!(parse_combo("Ctrl+Nonsense").is_err());
        assert!(parse_combo("A+B").is_err());
        assert!(parse_combo("F25").is_err());
    }

    #[test]
    fn adds_https_to_bare_addresses() {
        assert_eq!(website_address("example.com"), "https://example.com");
        assert_eq!(website_address(" http://a.b "), "http://a.b");
        assert_eq!(website_address("mailto:a@b.c"), "mailto:a@b.c");
    }
}
