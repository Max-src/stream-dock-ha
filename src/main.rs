#![windows_subsystem = "windows"]

//! Stream Dock HA: a grid of programmable buttons that lives next to the clock.
//!
//! The window only exists while it's open. When the app starts in the
//! background, just the tray icon runs, so nothing can flash on screen and
//! almost no memory is used until the window is asked for.

mod actions;
mod alerts;
mod confirm;
mod band;
mod bridge;
mod camera;
mod flashes;
mod frames;
mod ha;
mod icons;
mod media;
mod model;
mod monitor;
mod pad;
mod pointer_back;
mod recorder;
mod remote;
mod remote_mouse;
mod screen_off;
mod driver;
mod firmware;
mod screen_mouse;
mod render;
mod setup;
mod small_pad;
mod tray;
mod win;
mod ui;

use model::{lock, Shared, APP_NAME};
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

struct Options {
    hidden: bool,
    show: bool,
    screenshot: Option<PathBuf>,
}

fn parse_args() -> Options {
    let mut options = Options {
        hidden: false,
        show: false,
        screenshot: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--hidden" | "--tray" | "--background" => options.hidden = true,
            "--show" => options.show = true,
            "--screenshot" => options.screenshot = args.next().map(PathBuf::from),
            _ => {}
        }
    }
    options
}

fn show_error(text: &str) {
    let (text, title) = (crate::win::wide(text), crate::win::wide(APP_NAME));
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn main() {
    // From the setup file, before it shows its options: whether Mini Monitor's
    // driver is already on this PC (exit code 0) or not (1), by the app's own
    // check, so the setup can show that step as done.
    if std::env::args().skip(1).any(|arg| arg == "--driver-present") {
        std::process::exit(if driver::present() { 0 } else { 1 });
    }
    if recorder::run_as_helper() {
        return;
    }
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    if setup::run_if_asked() {
        return;
    }
    // From the setup file's "Mini Monitor's display driver" step: installs
    // Parsec's driver if it isn't there, then ends, without starting the app.
    if std::env::args().skip(1).any(|arg| arg == "--install-driver") {
        if let Err(why) = driver::install(&|_| {}) {
            show_error(&format!("Mini Monitor's display driver isn't installed: {why}. You can install it later from the Monitor tab."));
            std::process::exit(1);
        }
        return;
    }
    let options = parse_args();

    // A picture of the window for tests and docs, with nothing saved and
    // without touching a copy that may already be running near the clock.
    #[cfg(not(feature = "shots"))]
    if options.screenshot.is_some() {
        eprintln!("This build takes no pictures of its window: build it with the \"shots\" feature.");
        std::process::exit(2);
    }
    #[cfg(feature = "shots")]
    if let Some(path) = options.screenshot {
        let (shared, _opened) = Shared::new(model::load(true).cfg, true);
        if let Some(example) = std::env::var_os("STREAM_DOCK_HA_SHOT_ENTITIES") {
            ha::load_example(&shared, std::path::Path::new(&example));
        }
        // A made-up past for the graphs of Home Assistant sensors.
        if std::env::var_os("STREAM_DOCK_HA_SHOT_HISTORY").is_some() {
            ha::history::show_made_up(&shared);
        }
        // The alignment picture on a pad on screen needs no display driver.
        if std::env::var_os("STREAM_DOCK_HA_SHOT_PATTERN").is_some() && std::env::var_os("STREAM_DOCK_HA_SHOT_ON_SCREEN").is_some() {
            monitor::start_on_screen(shared.clone());
        }
        match std::env::var("STREAM_DOCK_HA_SHOT_STATS") {
            Ok(numbers) => band::show_made_up(&shared, &numbers),
            Err(_) => band::start_measuring(shared.clone()),
        }
        if let Err(e) = ui::open_window(shared, Some(path)) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    // A test copy, for end-to-end tests next to the person's own copy: its
    // settings and log stay in its folder, and it leaves the pads, the
    // shortcuts, the start-up list and the tray alone.
    let sandbox = model::sandbox_folder().is_some();

    // One icon is enough: a second start opens the window of the first. The
    // name is from the app's earlier name, Tray Dock, so a copy from before
    // the rename and one from after never drive the pad at the same time.
    let name = crate::win::wide(if sandbox { "Local\\StreamDockHASandbox" } else { "Local\\TrayDockSingleInstance" });
    let _instance = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // The window found that way would be the person's own copy's.
        if !options.hidden && !sandbox {
            tray::ask_running_copy_to_open();
        }
        return;
    }

    // Loaded only now, so a second copy that is about to close never touches
    // the settings of the one that's running.
    model::adopt_old_settings();
    let loaded = model::load(false);
    let cfg = loaded.cfg;
    let show_at_start = if sandbox {
        options.show
    } else {
        !options.hidden && (options.show || !cfg.start_hidden || !cfg.first_run_done)
    };
    let (shared, open_requests) = Shared::new(cfg, false);
    shared.keep_settings_file.store(!loaded.can_save, std::sync::atomic::Ordering::SeqCst);
    if let Some(problem) = loaded.problem {
        *lock(&shared.status) = problem;
    }
    // Made-up entities for a test copy, as for pictures.
    #[cfg(feature = "shots")]
    if let Some(example) = std::env::var_os("STREAM_DOCK_HA_SHOT_ENTITIES") {
        ha::load_example(&shared, std::path::Path::new(&example));
    }
    if sandbox {
        pad::note("test copy started: no pads, shortcuts, start-up entry, tray icon or Mini Monitor display");
    } else {
        tray::refresh_autostart();
        tray::start(shared.clone());
        pointer_back::start();
        pad::start(shared.clone());
        small_pad::start(shared.clone());
        // Mini Monitor on screen adds a display to Windows, which a test copy mustn't.
        monitor::start_on_screen(shared.clone());
    }
    crate::model::start_saver(shared.clone());
    band::start_measuring(shared.clone());
    bridge::start(shared.clone());
    ha::start(shared.clone());
    camera::start(shared.clone());
    media::watch_microphone(shared.clone());
    screen_off::watch(shared.clone());

    if show_at_start {
        let _ = shared.open_request.send(());
    }

    // Waits, using no processor time, until the window is asked for. Closing
    // the window comes back here while the icon carries on.
    while open_requests.recv().is_ok() {
        if let Err(e) = ui::open_window(shared.clone(), None) {
            show_error(&format!("The window couldn't open: {e}"));
        }
        // A test copy has no icon to open the window again from.
        if sandbox {
            return;
        }
        while open_requests.try_recv().is_ok() {}
    }
}
