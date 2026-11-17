//! The saved data (profiles, pages, keys) and the state shared between the
//! tray icon, which runs all the time, and the window, which comes and goes.

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

mod graphs;
mod ids;
mod keys;
pub mod layout;
mod meters;
mod pads;
mod profile;
mod config;
mod storage;
pub use graphs::*;
pub use ids::*;
pub use keys::*;
pub use meters::*;
pub use pads::*;
pub use profile::*;
pub use config::*;
pub use storage::*;

pub const APP_NAME: &str = "Stream Dock HA";

/// How far one click of a knob changes the brightness in the original.
pub const BRIGHTNESS_STEP: i32 = 5;
/// A pad's brightness until it's changed, in the original.
pub const DEFAULT_BRIGHTNESS: u8 = 25;

/// What loading the settings found.
pub struct Loaded {
    pub cfg: Config,
    /// Something to tell the person about.
    pub problem: Option<String>,
    /// False when the settings file is there but couldn't be read, perhaps
    /// because another program has it open. Saving would replace it.
    pub can_save: bool,
}

pub struct Shared {
    pub cfg: Mutex<Config>,
    pub status: Mutex<String>,
    pub hotkey_errors: Mutex<Vec<String>>,
    /// The window's handle while it exists, otherwise 0.
    pub window: AtomicIsize,
    pub open_request: Sender<()>,
    pub egui_ctx: Mutex<Option<egui::Context>>,
    pub pad_status: Mutex<PadStatus>,
    pub stats: Mutex<crate::band::Stats>,
    /// Recent readings of each band tile, for its graph.
    pub history: Mutex<crate::band::History>,
    /// Mini Monitor shows the alignment pattern instead of the display.
    pub alignment_pattern: AtomicBool,
    pub ha: crate::ha::Hub,
    /// Counts up whenever something Home Assistant's integration reports may
    /// have changed: settings, entity states, a pad's connection or screen, the
    /// microphone, a camera picture or a press. Anything that wants to follow
    /// those subscribes and decides for itself how often to look.
    pub changes: tokio::sync::watch::Sender<u64>,
    /// Alerts from Home Assistant covering a pad's keys, by pad id.
    pub alerts: Mutex<crate::alerts::Alerts>,
    /// What Home Assistant shows on single keys for a while, by pad id and cell.
    pub flashes: Mutex<crate::flashes::Flashes>,
    /// The link to Home Assistant's Stream Dock HA integration.
    pub bridge: crate::bridge::Link,
    /// When any pad was last pressed, for Home Assistant.
    pub last_press: Mutex<Option<std::time::SystemTime>>,
    /// A picture to store as the pad's startup picture, once confirmed.
    pub startup_picture: Mutex<Option<PathBuf>>,
    /// The key last pressed on the pad, to light it up on screen.
    pub pad_press: Mutex<Option<(usize, Instant)>>,
    /// The key last pressed on a virtual pad, with that pad's id, to light
    /// it up on screen.
    pub dashboard_press: Mutex<Option<(String, usize, Instant)>>,
    /// Pads shown on screen, by id, so Mini Monitor can run for them without
    /// the real pad.
    pub on_screen: Mutex<Vec<String>>,
    /// Pads to open or close on screen, by id, asked for from the tray.
    pub screen_requests: Mutex<Vec<String>>,
    /// The next window starts hidden, for a pad on screen on its own.
    pub open_hidden: AtomicBool,
    /// For `--screenshot` and tests: nothing is saved, registered or run.
    pub read_only: bool,
    /// The settings file couldn't be read, so it isn't written over this
    /// session; everything else works as usual.
    pub keep_settings_file: AtomicBool,
    /// Numbers each copy of the settings handed to a save, in the order the
    /// settings changed.
    save_order: AtomicU64,
    /// The number of the copy last written. Saves go one at a time, and an
    /// older copy is never written over a newer one.
    saved: Mutex<u64>,
    /// Saves asked for and not yet written, once the saver runs. Changes come
    /// in bursts (a slider, an automation), and one write covers them all.
    save_asked: Mutex<u64>,
    save_signal: std::sync::Condvar,
    saver_running: AtomicBool,
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    /// The state every part of the app shares, and the channel that asks for
    /// the window.
    pub fn new(cfg: Config, read_only: bool) -> (Arc<Shared>, Receiver<()>) {
        let (open_request, opened) = std::sync::mpsc::channel();
        let shared = Shared {
            cfg: Mutex::new(cfg),
            status: Mutex::new(String::new()),
            hotkey_errors: Mutex::new(Vec::new()),
            window: AtomicIsize::new(0),
            open_request,
            egui_ctx: Mutex::new(None),
            pad_status: Mutex::new(PadStatus::NotFound),
            stats: Default::default(),
            history: Default::default(),
            alignment_pattern: AtomicBool::new(false),
            ha: Default::default(),
            changes: tokio::sync::watch::channel(0).0,
            alerts: Default::default(),
            flashes: Default::default(),
            bridge: Default::default(),
            last_press: Mutex::new(None),
            startup_picture: Mutex::new(None),
            pad_press: Mutex::new(None),
            dashboard_press: Mutex::new(None),
            on_screen: Mutex::new(Vec::new()),
            screen_requests: Mutex::new(Vec::new()),
            open_hidden: AtomicBool::new(false),
            read_only,
            keep_settings_file: AtomicBool::new(false),
            save_order: AtomicU64::new(0),
            saved: Mutex::new(0),
            save_asked: Mutex::new(0),
            save_signal: std::sync::Condvar::new(),
            saver_running: AtomicBool::new(false),
        };
        (Arc::new(shared), opened)
    }

    pub fn set_status(&self, text: impl Into<String>) {
        *lock(&self.status) = text.into();
        self.repaint();
    }

    /// Says that something worth reporting changed, and redraws the window.
    pub fn touch(&self) {
        self.changes.send_modify(|count| *count = count.wrapping_add(1));
        self.repaint();
    }

    /// As `touch`, for changes that come in bursts, like Home Assistant's
    /// states: the window redraws once for the lot, a moment later.
    pub fn touch_soon(&self) {
        self.changes.send_modify(|count| *count = count.wrapping_add(1));
        if let Some(ctx) = lock(&self.egui_ctx).as_ref() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    pub fn repaint(&self) {
        if let Some(ctx) = lock(&self.egui_ctx).as_ref() {
            ctx.request_repaint();
        }
    }

    /// Saves a configuration that's in use right now, without locking it again.
    /// Whether saving is off: in a test, or to protect a settings file that
    /// couldn't be read.
    fn no_saving(&self) -> bool {
        self.read_only || self.keep_settings_file.load(Ordering::SeqCst)
    }

    pub fn save_now_with(&self, cfg: &Config) {
        // Settings that change count even when they aren't written, as in a test.
        self.touch();
        if self.no_saving() || self.ask_saver() {
            return;
        }
        // Called with the settings locked, so this number follows their order.
        let number = self.save_order.fetch_add(1, Ordering::SeqCst) + 1;
        self.write(cfg, number);
    }

    fn write(&self, cfg: &Config, number: u64) {
        let mut written = lock(&self.saved);
        if number < *written {
            return;
        }
        match save(cfg) {
            Ok(()) => *written = number,
            Err(e) => self.set_status(format!("Couldn't save your buttons: {e}")),
        }
    }

    pub fn save_now(&self) {
        self.touch();
        if self.no_saving() || self.ask_saver() {
            return;
        }
        self.write_current();
    }

    /// Writes the settings at once, as before the app ends.
    pub fn save_before_quitting(&self) {
        if !self.no_saving() {
            self.write_current();
        }
    }

    /// Hands a save to the saver, if it runs: false when there's none.
    fn ask_saver(&self) -> bool {
        if !self.saver_running.load(Ordering::SeqCst) {
            return false;
        }
        *lock(&self.save_asked) += 1;
        self.save_signal.notify_one();
        true
    }

    fn write_current(&self) {
        let (cfg, number) = {
            let cfg = lock(&self.cfg);
            (cfg.clone(), self.save_order.fetch_add(1, Ordering::SeqCst) + 1)
        };
        self.write(&cfg, number);
    }
}

/// Writes the settings a moment after they change, off the window's thread,
/// once for each burst of changes.
pub fn start_saver(shared: Arc<Shared>) {
    /// How long the settings must stay unchanged before they're written.
    const QUIET: std::time::Duration = std::time::Duration::from_millis(400);
    shared.saver_running.store(true, Ordering::SeqCst);
    std::thread::spawn(move || loop {
        let mut asked = lock(&shared.save_asked);
        while *asked == 0 {
            asked = shared.save_signal.wait(asked).unwrap_or_else(|e| e.into_inner());
        }
        // Waits until the changes stop for a moment.
        loop {
            let seen = *asked;
            drop(asked);
            std::thread::sleep(QUIET);
            asked = lock(&shared.save_asked);
            if *asked == seen {
                break;
            }
        }
        *asked = 0;
        drop(asked);
        shared.write_current();
    });
}

#[cfg(test)]
mod tests;
