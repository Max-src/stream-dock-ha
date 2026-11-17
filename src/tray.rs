//! The icon near the clock. It runs on its own thread with its own hidden
//! message window, so it works whether or not the main window exists, and
//! that's also where the keyboard shortcuts arrive.

use crate::actions::{self, parse_combo};
use crate::win::wide;
use crate::model::{lock, Shared, APP_NAME};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Kept from the app's earlier name, Tray Dock, so a newer copy can still
/// find an older one that is running and open its window.
const CLASS_NAME: &str = "TrayDockTrayWindow";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "StreamDockHA";
/// The start-up entries under the earlier names. They move to the new one.
const OLD_RUN_VALUES: [&str; 2] = ["StreamDockMax", "TrayDock"];

const WM_TRAY: u32 = WM_APP + 1;
const WM_RELOAD: u32 = WM_APP + 2;
const WM_SHOW_REQUEST: u32 = WM_APP + 3;
const WM_QUIT_REQUEST: u32 = WM_APP + 4;

const ID_OPEN: u32 = 1;
const ID_AUTOSTART: u32 = 2;
const ID_START_HIDDEN: u32 = 3;
const ID_QUIT: u32 = 4;
const ID_MINI_MONITOR: u32 = 5;
/// One menu entry for each pad on screen, in the order of `Config::pads_in_use`.
const ID_ON_SCREEN_BASE: u32 = 50;
const ID_PROFILE_BASE: u32 = 100;

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
static TRAY_WINDOW: AtomicIsize = AtomicIsize::new(0);
static ICON: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
/// (profile, page, key) for each registered shortcut; its id is the index + 1.
static HOTKEYS: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

/// Draws the app's icon: a small grid of buttons on a blue tile.
pub fn icon_rgba(size: usize) -> Vec<u8> {
    const SAMPLES: usize = 4;
    let mut out = Vec::with_capacity(size * size * 4);
    let s = size as f32;
    let inside_rounded = |x: f32, y: f32, l: f32, t: f32, r: f32, b: f32, rad: f32| {
        let dx = (l + rad - x).max(x - (r - rad)).max(0.0);
        let dy = (t + rad - y).max(y - (b - rad)).max(0.0);
        dx * dx + dy * dy <= rad * rad && x >= l && x <= r && y >= t && y <= b
    };
    for py in 0..size {
        for px in 0..size {
            let (mut tile, mut button) = (0u32, 0u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / s;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / s;
                    if inside_rounded(x, y, 0.0, 0.0, 1.0, 1.0, 0.22) {
                        tile += 1;
                        let hit = (0..3).any(|c| {
                            (0..2).any(|r| {
                                let l = 0.15 + c as f32 * 0.25;
                                let t = 0.24 + r as f32 * 0.29;
                                inside_rounded(x, y, l, t, l + 0.20, t + 0.23, 0.05)
                            })
                        });
                        if hit {
                            button += 1;
                        }
                    }
                }
            }
            let total = (SAMPLES * SAMPLES) as f32;
            let on = button as f32 / tile.max(1) as f32;
            let mix = |bg: f32| (bg + (255.0 - bg) * on) as u8;
            out.extend_from_slice(&[
                mix(52.0),
                mix(112.0),
                mix(236.0),
                (tile as f32 / total * 255.0) as u8,
            ]);
        }
    }
    out
}

fn make_icon() -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as usize;
    let rgba = icon_rgba(size);
    let mut bgra = rgba.clone();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let stride = size.div_ceil(32) * 4;
    let mask = vec![0u8; stride * size];
    unsafe {
        CreateIcon(
            std::ptr::null_mut(),
            size as i32,
            size as i32,
            1,
            32,
            mask.as_ptr(),
            bgra.as_ptr(),
        )
    }
}

unsafe fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data
}

unsafe fn set_icon(hwnd: HWND, operation: u32) {
    let mut data = icon_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = ICON.load(Ordering::SeqCst) as HICON;
    let tip = match SHARED.get() {
        Some(s) => format!("{APP_NAME} ({})", lock(&s.cfg).main_profile().name),
        None => APP_NAME.to_string(),
    };
    for (slot, unit) in data.szTip.iter_mut().take(127).zip(tip.encode_utf16()) {
        *slot = unit;
    }
    Shell_NotifyIconW(operation, &data);
}

fn register_hotkeys(hwnd: HWND, shared: &Shared) {
    let mut table = lock(&HOTKEYS);
    for id in 1..=table.len() {
        unsafe { UnregisterHotKey(hwnd, id as i32) };
    }
    table.clear();

    let mut errors = Vec::new();
    let cfg = lock(&shared.cfg);
    let pi = cfg.main_index();
    for (page_i, page) in cfg.main_profile().pages.iter().enumerate() {
        for (key_i, key) in page.keys.iter().enumerate() {
            if key.hotkey.trim().is_empty() {
                continue;
            }
            let label = if key.title.is_empty() { "A key" } else { &key.title };
            let combo = match parse_combo(&key.hotkey) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(format!("{label}: {e}"));
                    continue;
                }
            };
            let is_function_key = (VK_F1..=VK_F24).contains(&combo.vk);
            if !(combo.ctrl || combo.alt || combo.shift || combo.win || is_function_key) {
                errors.push(format!(
                    "{label}: {} would take that key away from everything you type, so add Ctrl or Alt",
                    key.hotkey
                ));
                continue;
            }
            let mut mods = MOD_NOREPEAT;
            if combo.ctrl {
                mods |= MOD_CONTROL;
            }
            if combo.alt {
                mods |= MOD_ALT;
            }
            if combo.shift {
                mods |= MOD_SHIFT;
            }
            if combo.win {
                mods |= MOD_WIN;
            }
            let id = table.len() as i32 + 1;
            if unsafe { RegisterHotKey(hwnd, id, mods, combo.vk as u32) } != 0 {
                table.push((pi, page_i, key_i));
            } else {
                errors.push(format!(
                    "{label}: {} is already used by Windows or another program",
                    key.hotkey
                ));
            }
        }
    }
    drop(cfg);
    for error in &errors {
        crate::pad::note(&format!("shortcut problem: {error}"));
    }
    *lock(&shared.hotkey_errors) = errors;
    shared.repaint();
}

/// Opens or closes a pad on screen. If the window isn't running, it starts
/// hidden, so only the pad appears.
pub fn toggle_on_screen(shared: &Shared, pad_id: &str) {
    lock(&shared.screen_requests).push(pad_id.to_string());
    if shared.window.load(Ordering::SeqCst) == 0 {
        shared.open_hidden.store(true, Ordering::SeqCst);
        let _ = shared.open_request.send(());
    } else {
        shared.repaint();
    }
}

/// Opens the window, or brings it forward if it's already open.
fn open_window(shared: &Shared) {
    let hwnd = shared.window.load(Ordering::SeqCst) as HWND;
    if hwnd.is_null() {
        let _ = shared.open_request.send(());
    } else {
        unsafe {
            ShowWindowAsync(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
            crate::win::bring_to_front(hwnd);
            SetForegroundWindow(hwnd);
        }
    }
}

fn fire_hotkey(shared: &Arc<Shared>, id: usize) {
    let Some(&(pi, page, key)) = lock(&HOTKEYS).get(id.wrapping_sub(1)) else {
        return;
    };
    let cfg = lock(&shared.cfg);
    let Some(k) = cfg.profiles.get(pi).and_then(|p| p.pages.get(page)).and_then(|p| p.keys.get(key))
    else {
        return;
    };
    let (name, count) = (k.title.clone(), k.actions.len());
    let Some(pad_id) = cfg.pad_of_profile(pi).map(|entry| entry.id.clone()) else { return };
    drop(cfg);
    crate::pad::note(&format!("shortcut for key \"{name}\" with {count} steps"));
    // The same press as on the pad, so Home Assistant hears of it too.
    if let Err(problem) = actions::press(shared, &pad_id, key, Some((pi, page)), actions::PressSource::Hotkey) {
        crate::pad::note(&format!("shortcut refused: {problem}"));
    }
}

/// Removes the icon and ends the whole app. The buttons are saved first.
pub fn quit() -> ! {
    if let Some(shared) = SHARED.get() {
        shared.save_before_quitting();
    }
    crate::pad::finish();
    let hwnd = TRAY_WINDOW.load(Ordering::SeqCst) as HWND;
    if !hwnd.is_null() {
        unsafe {
            let data = icon_data(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
    std::process::exit(0);
}

unsafe fn append(menu: HMENU, id: u32, text: &str, checked: bool) {
    let text = wide(text);
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    AppendMenuW(menu, flags, id as usize, text.as_ptr());
}

unsafe fn show_menu(hwnd: HWND, shared: &Arc<Shared>) {
    let (names, running, start_hidden, mini_monitor, pads) = {
        let cfg = lock(&shared.cfg);
        let names: Vec<String> = cfg.profiles.iter().map(|p| p.name.clone()).collect();
        let running: Vec<Option<usize>> = cfg.pads_in_use().map(|entry| cfg.pad_profile_of(&entry.id)).collect();
        let pads: Vec<(String, String)> = cfg.pads_in_use().map(|entry| (entry.id.clone(), entry.name.clone())).collect();
        (names, running, cfg.start_hidden, cfg.mini_monitor, pads)
    };
    let open_on_screen = lock(&shared.on_screen).clone();
    let menu = CreatePopupMenu();
    append(menu, ID_OPEN, &format!("Open {APP_NAME}"), false);
    SetMenuDefaultItem(menu, ID_OPEN, 0);
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    let profiles = CreatePopupMenu();
    for (i, name) in names.iter().enumerate() {
        append(profiles, ID_PROFILE_BASE + i as u32, name, running.contains(&Some(i)));
    }
    let label = wide("Profile");
    AppendMenuW(menu, MF_POPUP, profiles as usize, label.as_ptr());
    append(menu, ID_MINI_MONITOR, "Mini Monitor", mini_monitor);
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    // Every pad that's set up, plugged in or not.
    for (at, (pad_id, name)) in pads.iter().enumerate().take((ID_PROFILE_BASE - ID_ON_SCREEN_BASE) as usize) {
        append(menu, ID_ON_SCREEN_BASE + at as u32, &format!("{name} on screen"), open_on_screen.contains(pad_id));
    }
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    append(menu, ID_AUTOSTART, "Start with Windows", autostart_enabled());
    append(menu, ID_START_HIDDEN, "Start without the window", start_hidden);
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    append(menu, ID_QUIT, "Quit", false);

    let mut pos = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pos);
    // Windows only closes the menu on an outside click if this window is in front.
    SetForegroundWindow(hwnd);
    let chosen = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
        pos.x,
        pos.y,
        0,
        hwnd,
        std::ptr::null(),
    ) as u32;
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);

    match chosen {
        ID_OPEN => open_window(shared),
        ID_AUTOSTART => {
            if let Err(e) = set_autostart(!autostart_enabled()) {
                shared.set_status(e);
            }
            shared.repaint();
        }
        ID_START_HIDDEN => {
            {
                let mut cfg = lock(&shared.cfg);
                cfg.start_hidden = !cfg.start_hidden;
            }
            shared.save_now();
            shared.repaint();
        }
        ID_QUIT => quit(),
        ID_MINI_MONITOR => {
            {
                let mut cfg = lock(&shared.cfg);
                cfg.mini_monitor = !cfg.mini_monitor;
            }
            shared.save_now();
            crate::pad::wake();
            shared.repaint();
        }
        id if (ID_ON_SCREEN_BASE..ID_PROFILE_BASE).contains(&id) => {
            if let Some((pad_id, _)) = pads.get((id - ID_ON_SCREEN_BASE) as usize) {
                toggle_on_screen(shared, pad_id);
            }
        }
        id if id >= ID_PROFILE_BASE => {
            {
                let mut cfg = lock(&shared.cfg);
                let i = (id - ID_PROFILE_BASE) as usize;
                if i < cfg.profiles.len() {
                    cfg.run_profile(i);
                }
            }
            shared.save_now();
            reload_now(hwnd, shared);
        }
        _ => {}
    }
}

unsafe fn reload_now(hwnd: HWND, shared: &Shared) {
    register_hotkeys(hwnd, shared);
    set_icon(hwnd, NIM_MODIFY);
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(shared) = SHARED.get() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        // Windows is shutting down or signing out: a save that was waiting
        // for the settings to settle is written now.
        WM_ENDSESSION if wp != 0 => {
            shared.save_before_quitting();
            0
        }
        WM_TRAY => {
            match (lp & 0xffff) as u32 {
                WM_LBUTTONUP => open_window(shared),
                WM_RBUTTONUP => show_menu(hwnd, shared),
                _ => {}
            }
            0
        }
        WM_SHOW_REQUEST => {
            open_window(shared);
            0
        }
        WM_QUIT_REQUEST => quit(),
        WM_RELOAD => {
            reload_now(hwnd, shared);
            0
        }
        WM_HOTKEY => {
            fire_hotkey(shared, wp);
            0
        }
        // Explorer restarted, which wipes every icon near the clock.
        m if m == TASKBAR_CREATED.load(Ordering::SeqCst) && m != 0 => {
            set_icon(hwnd, NIM_ADD);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Starts the icon on its own thread.
pub fn start(shared: Arc<Shared>) {
    let _ = SHARED.set(shared);
    std::thread::spawn(|| unsafe {
        let class = wide(CLASS_NAME);
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(window_proc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        TRAY_WINDOW.store(hwnd as isize, Ordering::SeqCst);
        ICON.store(make_icon() as isize, Ordering::SeqCst);
        let name = wide("TaskbarCreated");
        TASKBAR_CREATED.store(RegisterWindowMessageW(name.as_ptr()), Ordering::SeqCst);

        set_icon(hwnd, NIM_ADD);
        if let Some(shared) = SHARED.get() {
            register_hotkeys(hwnd, shared);
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

/// Asks the icon to pick up changed shortcuts or a changed profile.
pub fn reload() {
    let hwnd = TRAY_WINDOW.load(Ordering::SeqCst) as HWND;
    if !hwnd.is_null() {
        unsafe { PostMessageW(hwnd, WM_RELOAD, 0, 0) };
    }
}

/// Before installing over it or removing it: asks a running copy to quit.
pub fn ask_running_copy_to_quit() {
    let class = wide(CLASS_NAME);
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_QUIT_REQUEST, 0, 0);
        }
    }
}

/// When the app is started a second time: asks the running copy to open its window.
pub fn ask_running_copy_to_open() {
    let class = wide(CLASS_NAME);
    unsafe {
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if !hwnd.is_null() {
            // Lets the running copy come to the front.
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            AllowSetForegroundWindow(pid);
            PostMessageW(hwnd, WM_SHOW_REQUEST, 0, 0);
        }
    }
}

fn read_run_value(name: &str) -> Option<String> {
    let (key, value) = (wide(RUN_KEY), wide(name));
    let mut buffer = [0u16; 1024];
    let mut bytes = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let len = (bytes as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..len]))
}

fn write_run_value(name: &str, data: &str) -> Result<(), String> {
    let (key, value, data) = (wide(RUN_KEY), wide(name), wide(data));
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err("Windows refused to change the start-up setting".into())
    }
}

fn delete_run_value(name: &str) -> Result<(), String> {
    let (key, value) = (wide(RUN_KEY), wide(name));
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr()) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err("Windows refused to change the start-up setting".into())
    }
}

fn run_command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\" --hidden", exe.display()))
}

/// A test copy never touches Windows' start-up list: it's shared with the
/// copy the person uses.
fn refuse_in_sandbox() -> Result<(), String> {
    match crate::model::sandbox_folder() {
        Some(_) => Err("A test copy doesn't change what starts with Windows".into()),
        None => Ok(()),
    }
}

/// Start with Windows for a copy somewhere else, as the installer needs.
pub fn set_autostart_for(program: &std::path::Path) -> Result<(), String> {
    refuse_in_sandbox()?;
    write_run_value(RUN_VALUE, &format!("\"{}\" --hidden", program.display()))
}

pub fn autostart_enabled() -> bool {
    read_run_value(RUN_VALUE).is_some()
}

/// Windows starts the app with `--hidden`, so only the icon appears at log-in.
pub fn set_autostart(on: bool) -> Result<(), String> {
    refuse_in_sandbox()?;
    if on {
        write_run_value(RUN_VALUE, &run_command()?)
    } else {
        delete_run_value(RUN_VALUE)
    }
}

/// If the app was moved, points the start-up entry at its new place. An entry
/// from before the rename becomes one under the new name, so the app doesn't
/// start twice at log-in.
pub fn refresh_autostart() {
    if refuse_in_sandbox().is_err() {
        return;
    }
    if OLD_RUN_VALUES.iter().any(|old| read_run_value(old).is_some()) {
        if let Ok(wanted) = run_command() {
            if write_run_value(RUN_VALUE, &wanted).is_ok() {
                for old in OLD_RUN_VALUES {
                    let _ = delete_run_value(old);
                }
            }
        }
    }
    if let (Some(current), Ok(wanted)) = (read_run_value(RUN_VALUE), run_command()) {
        if current != wanted {
            let _ = write_run_value(RUN_VALUE, &wanted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_with_windows_entry_can_be_added_and_removed() {
        let name = "StreamDockHATestEntry";
        delete_run_value(name).unwrap();
        assert_eq!(read_run_value(name), None);
        write_run_value(name, "\"C:\\a b\\c.exe\" --hidden").unwrap();
        assert_eq!(read_run_value(name).as_deref(), Some("\"C:\\a b\\c.exe\" --hidden"));
        delete_run_value(name).unwrap();
        assert_eq!(read_run_value(name), None);
    }

    #[test]
    fn icon_has_the_right_size_and_rounded_corners() {
        let rgba = icon_rgba(32);
        assert_eq!(rgba.len(), 32 * 32 * 4);
        assert_eq!(rgba[3], 0);
        let middle = (16 * 32 + 3) * 4 + 3;
        assert_eq!(rgba[middle], 255);
    }
}
