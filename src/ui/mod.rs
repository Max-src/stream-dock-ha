//! The main window. It holds no state of its own that matters: everything
//! lives in `Shared`, so closing it loses nothing and the icon carries on.

use crate::actions;
use crate::band::{self, Face};
use crate::frames;
use crate::ha::{self, Entities};
use crate::icons;
use crate::recorder::{self, Recorded};
use crate::small_pad;
use crate::model::{
    apply_moves, color_of_state, lock, small_cell_has_screen, Cover, small_cell_of_key, small_knob_cells, small_knob_of_cell, Action, BRIGHTNESS_STEP, BandItem, Config, DockKey, HaAction, KeyShape, KeyStyle, MediaKey, MeterLevel, MeterStyle, MonitorArea, StateColor, MAX_METER_LEVELS, METER_PRESETS,
    KnobUse, PadKind, PictureFit, Shared, TextPlace, ThemeChoice, APP_NAME, MIC_ICON, SMALL_KNOBS,
};
use crate::render::{self, Look};
use crate::tray;
use eframe::egui::{
    self, vec2, Align, Align2, CentralPanel, Color32, ComboBox, CornerRadius, DragValue, FontId,
    Frame, Id, LayerId, Layout, Margin, Modal, Order, Panel, Rect, RichText, ScrollArea, Sense,
    Stroke, StrokeKind, TextEdit, TextureHandle, TextureOptions, Theme, Ui, Vec2, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

mod style;
mod widgets;
mod library;
mod pad_view;
mod entities;
mod inspector;
mod side_tabs;
mod top_bar;
mod dialogs;
mod selection;
mod virtual_pad;
mod pad_editor;
use style::*;
use pad_view::*;
use selection::*;
use widgets::*;
use library::*;
use entities::*;
pub use style::apply_theme;
pub use widgets::save_png;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Actions,
    Look,
    Monitor,
    Settings,
}

/// What a chosen entity goes to.
#[derive(Clone, Copy)]
enum BrowserTarget {
    Step(Place, usize),
    Band(usize),
    NewKeys,
}

#[derive(Clone, Copy, PartialEq)]
enum Recording {
    Hotkey(usize),
    Step(usize, usize),
}

#[derive(Clone, PartialEq)]
enum Confirm {
    DeleteProfile,
    DeletePage,
    DeleteKey(usize),
    /// The keys chosen together.
    DeleteSelection,
    Quit,
    /// Set a whole knob to one use, over steps the person chose themselves.
    ReplaceKnob(usize, KnobUse),
    /// Delete a virtual pad, by id, with its profiles.
    RemovePad(String),
}


#[derive(Clone, Copy)]
struct Place {
    profile: usize,
    page: usize,
    key: usize,
    /// Which profile that was, so a file chosen after the profiles changed
    /// doesn't land on another one.
    stamp: u64,
}

/// What identifies a profile while a file is being chosen for it.
fn profile_stamp(profile: &crate::model::Profile) -> u64 {
    hash_of(&(&profile.name, &profile.pad_id, profile.pages.len()))
}

enum Picked {
    Program(Place, usize, PathBuf),
    Image(Place, PathBuf),
    BandPicture(usize, PathBuf),
    HaImage(Place, usize, bool, PathBuf),
    StartupPicture(PathBuf),
    /// The profile, by name, and the file.
    SaveProfile(String, PathBuf),
    OpenProfile(PathBuf),
}

/// Dragged from the list onto a key.
#[derive(Clone, Copy)]
struct NewAction(usize);
/// A key being dragged to another cell.
#[derive(Clone, Copy)]
struct MoveKey {
    /// The profile and page it was picked up on: a drag that outlives them,
    /// as when a pad press changes the page mid-drag, is dropped.
    place: (usize, usize),
    cell: usize,
    /// Where on the key it was picked up, so it doesn't jump to the pointer.
    grab: Vec2,
}

/// A key that's been cut or copied. It stays here while profiles and pads
/// change, which is how keys travel between them.
#[derive(Clone)]
struct Clip {
    /// Each key with its row and column from the top left of what was copied.
    keys: Vec<(usize, usize, DockKey)>,
    /// Cut keys keep their shortcuts for their first paste: they're moving, not doubling.
    cut: bool,
}

struct DockApp {
    shared: Arc<Shared>,
    shot: Option<PathBuf>,
    frames: u32,
    selected: Option<usize>,
    selected_tile: Option<usize>,
    clip: Option<Clip>,
    ask_no_more: bool,
    /// Keys away from their cell on screen, and how far.
    glide: HashMap<usize, Vec2>,
    /// The profile and page `glide` belongs to.
    glide_place: (usize, usize),
    icon_search: String,
    /// The key drawn on top of the others: dragged, or landing after a drop.
    carried: Option<usize>,
    tab: Tab,
    /// Keys and tiles, as drawn for the pad.
    /// Pictures drawn for keys and band tiles, with the frame each was last used in.
    drawn: HashMap<u64, (TextureHandle, u32)>,
    browser: Option<EntityBrowser>,
    open_home_assistant: bool,
    theme: ThemeChoice,
    /// Lining up the gaps, with the test picture on the pad.
    adjusting: bool,
    font_search: String,
    ha_token_input: String,
    /// A startup picture waiting for the person to confirm it.
    startup_candidate: Option<PathBuf>,
    picked_tx: Sender<Picked>,
    picked_rx: Receiver<Picked>,
    recording: Option<Recording>,
    /// Several keys chosen together, with `selected` among them, and the
    /// profile and page they're on.
    many: Vec<usize>,
    many_at: (usize, usize),
    /// Where a box for choosing keys started, while it's being drawn.
    boxing: Option<egui::Pos2>,
    /// Pads shown on screen in windows of their own, by id.
    on_screen: Vec<String>,
    /// The latest Mini Monitor picture shown on a pad on screen, by pad
    /// (false for the 293S, true for the AKP03E), with its version.
    monitor_textures: HashMap<bool, (u64, TextureHandle)>,
    /// Pads on screen whose window corners have been tidied, by id.
    clean_cornered: Vec<String>,
    /// A pad on screen being resized from an edge.
    pad_resize: Option<virtual_pad::PadResize>,
    /// Installing it, from the Monitor tab.
    driver_step: Arc<std::sync::Mutex<crate::driver::Step>>,
    /// Where Mini Monitor on pads on screen takes the mouse, gathered each frame.
    mouse_zones: Vec<crate::screen_mouse::Zone>,
    /// Started from the tray for a pad on screen alone, without the window.
    started_hidden: bool,
    /// Where the window goes back to after starting off-screen.
    home: egui::Pos2,
    /// Presses on pads on screen, run once the frame lets go of the settings.
    pending_presses: Vec<(String, usize)>,
    /// The profile and page a recording started on.
    recording_at: Option<(usize, usize)>,
    /// Quit was chosen: it happens once this frame lets go of the settings.
    quitting: bool,
    renaming: Option<String>,
    /// A pad being renamed in the Pads section: its id and the new name.
    renaming_pad: Option<(String, String)>,
    /// The window adding or changing a virtual pad.
    pad_editor: Option<pad_editor::PadEditor>,
    confirm: Option<Confirm>,
    seen: Config,
    changed_at: Option<Instant>,
    hotkeys_seen: Vec<String>,
    show_hint: bool,
}

fn on_a_screen(x: f32, y: f32) -> bool {
    let metric = |m| unsafe { GetSystemMetrics(m) } as f32;
    let (left, top) = (metric(SM_XVIRTUALSCREEN), metric(SM_YVIRTUALSCREEN));
    let (width, height) = (metric(SM_CXVIRTUALSCREEN), metric(SM_CYVIRTUALSCREEN));
    x >= left - 40.0 && y >= top - 10.0 && x < left + width - 80.0 && y < top + height - 60.0
}

/// Opens the window and returns when it's closed.
pub fn open_window(shared: Arc<Shared>, shot: Option<PathBuf>) -> Result<(), String> {
    let saved = if shot.is_none() { lock(&shared.cfg).window } else { None };
    // Asked for from the tray for a pad on screen alone: the window stays hidden.
    let hidden = shot.is_none() && shared.open_hidden.swap(false, Ordering::SeqCst);
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        // Lets a pad on screen have see-through corners. This window is
        // painted edge to edge, so it looks the same.
        .with_transparent(true)
        .with_inner_size([1060.0, 720.0])
        .with_min_inner_size([900.0, 620.0])
        .with_icon(Arc::new(egui::IconData {
            rgba: tray::icon_rgba(64),
            width: 64,
            height: 64,
        }));
    if let Some([x, y, w, h]) = saved {
        viewport = viewport.with_inner_size([w, h]);
        if on_a_screen(x, y) {
            viewport = viewport.with_position([x, y]);
        }
    }
    if hidden {
        // Off-screen rather than hidden: eframe skips a hidden window with no
        // window of its own showing yet, so the pad would never be drawn. It
        // hides once the pad is up, then goes back to its place.
        viewport = viewport.with_position([-32000.0, -32000.0]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let first_shown = {
        let mut cfg = lock(&shared.cfg);
        let first = !cfg.first_run_done;
        cfg.first_run_done = true;
        first
    };
    shared.save_now();

    let app_shared = shared.clone();
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            if let Ok(handle) = cc.window_handle() {
                if let RawWindowHandle::Win32(win) = handle.as_raw() {
                    app_shared.window.store(win.hwnd.get(), Ordering::SeqCst);
                }
            }
            *lock(&app_shared.egui_ctx) = Some(cc.egui_ctx.clone());
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(ICON_FONT.into(), Arc::new(egui::FontData::from_static(icons::FONT)));
            fonts.families.insert(egui::FontFamily::Name(ICON_FONT.into()), vec![ICON_FONT.into()]);
            cc.egui_ctx.set_fonts(fonts);
            apply_theme(&cc.egui_ctx, lock(&app_shared.cfg).theme);
            let mut app = DockApp::new(app_shared, shot, first_shown);
            app.started_hidden = hidden;
            app.home = saved.map_or(egui::pos2(120.0, 120.0), |[x, y, _, _]| egui::pos2(x, y));
            Ok(Box::new(app))
        }),
    );

    shared.window.store(0, Ordering::SeqCst);
    *lock(&shared.egui_ctx) = None;
    shared.save_now();
    result.map_err(|e| e.to_string())
}

impl DockApp {
    fn new(shared: Arc<Shared>, shot: Option<PathBuf>, show_hint: bool) -> Self {
        let theme = lock(&shared.cfg).theme;
        let (picked_tx, picked_rx) = channel();
        let seen = lock(&shared.cfg).clone();
        let hotkeys_seen = hotkey_list(&seen);
        DockApp {
            shared,
            shot,
            frames: 0,
            selected: None,
            selected_tile: None,
            glide: HashMap::new(),
            glide_place: (0, 0),
            icon_search: String::new(),
            carried: None,
            tab:Tab::Actions,
            clip: None,
            ask_no_more: false,
            drawn: HashMap::new(),
            browser: None,
            open_home_assistant: false,
            theme,
            adjusting: false,
            font_search: String::new(),
            ha_token_input: String::new(),
            startup_candidate: None,
            picked_tx,
            picked_rx,
            recording: None,
            many: Vec::new(),
            many_at: (0, 0),
            boxing: None,
            on_screen: Vec::new(),
            pending_presses: Vec::new(),
            monitor_textures: HashMap::new(),
            clean_cornered: Vec::new(),
            pad_resize: None,
            mouse_zones: Vec::new(),
            driver_step: Arc::new(std::sync::Mutex::new(crate::driver::Step::Idle)),
            started_hidden: false,
            home: egui::pos2(120.0, 120.0),
            recording_at: None,
            quitting: false,
            renaming: None,
            renaming_pad: None,
            pad_editor: None,
            confirm: None,
            seen,
            changed_at: None,
            hotkeys_seen,
            show_hint,
        }
    }

    fn capture_recording(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        let Some(recording) = self.recording else {
            self.recording_at = None;
            if recorder::is_active() {
                recorder::stop();
            }
            return;
        };
        let (at_profile, at_page) = *self.recording_at.get_or_insert((cfg.active_profile, cfg.profile().active_page));
        // Keys are only caught while someone is looking at the Record button.
        if ctx.input(|i| i.viewport().focused == Some(false)) {
            self.recording = None;
            recorder::stop();
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        let Some(result) = recorder::take() else { return };
        self.recording = None;
        self.recording_at = None;
        let Recorded::Keys(text) = result else { return };
        // The key it was for, even if the page changed or it has gone since.
        let key = cfg.profiles.get_mut(at_profile).and_then(|p| p.pages.get_mut(at_page)).and_then(|page| {
            let cell = match recording {
                Recording::Hotkey(i) | Recording::Step(i, _) => i,
            };
            page.keys.get_mut(cell)
        });
        let Some(key) = key else { return };
        match recording {
            Recording::Hotkey(_) => key.hotkey = text,
            Recording::Step(_, step) => {
                if let Some(Action::Shortcut { keys }) = key.actions.get_mut(step) {
                    *keys = text;
                }
            }
        }
    }

    /// Saves shortly after a change and tells the icon about new shortcuts.
    fn track_changes(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        if self.shared.read_only {
            return;
        }
        let rect = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        // Only where it's really shown: not hidden, nor off-screen at a start for a pad alone.
        let shown = ctx.input(|i| i.viewport().visible()).unwrap_or(true) && !(self.started_hidden && self.frames <= 3);
        if let (Some(outer), Some(inner), true) = (rect.0, rect.1, shown) {
            cfg.window = Some([
                outer.min.x.round(),
                outer.min.y.round(),
                inner.width().round(),
                inner.height().round(),
            ]);
        }
        if *cfg != self.seen {
            self.seen = cfg.clone();
            self.changed_at = Some(Instant::now());
            crate::pad::wake();
            // What the pads show changes now; the save comes a moment later.
            self.shared.touch();
            // Shortcuts can only have changed with the settings.
            let hotkeys = hotkey_list(cfg);
            if hotkeys != self.hotkeys_seen {
                self.hotkeys_seen = hotkeys;
                tray::reload();
            }
        }
        if let Some(at) = self.changed_at {
            if at.elapsed().as_secs_f32() > 0.8 {
                self.changed_at = None;
                self.shared.save_now_with(cfg);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }
    }

    /// The window's picture and the set-ups before it, for checks and the
    /// README's pictures. Left out of the setup file's build.
    #[cfg(feature = "shots")]
    fn take_picture_when_ready(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else { return };
        // Lets a check picture show the key editor or the settings as well.
        if self.frames == 1 {
            if let Ok(which) = std::env::var("STREAM_DOCK_HA_SHOT_ON_SCREEN") {
                // Drawn inside the main window, so the test picture shows it.
                ctx.set_embed_viewports(true);
                let cfg = lock(&self.shared.cfg);
                let of_kind = |kind: PadKind| -> Vec<String> { cfg.pads_in_use().filter(|entry| entry.kind == kind).map(|entry| entry.id.clone()).collect() };
                self.on_screen = match which.as_str() {
                    "main" => of_kind(PadKind::Main).into_iter().take(1).collect(),
                    "akp03e" => of_kind(PadKind::Akp03e).into_iter().take(1).collect(),
                    "dashboard" => of_kind(PadKind::Dashboard),
                    id if cfg.pad(id).is_some() => vec![id.to_string()],
                    _ => cfg.pads_in_use().map(|entry| entry.id.clone()).collect(),
                };
            }
            if let Some(cells) = std::env::var("STREAM_DOCK_HA_SHOT_CHOOSE").ok() {
                let shared = self.shared.clone();
                let cfg = lock(&shared.cfg);
                let cells: Vec<usize> = cells.split(',').filter_map(|cell| cell.trim().parse().ok()).collect();
                self.box_select(&cfg, cells, false);
            }
            if let Some(i) = std::env::var("STREAM_DOCK_HA_SHOT_KEY").ok().and_then(|v| v.parse().ok()) {
                self.selected = Some(i);
                self.selected_tile = None;
            }
            if let Some(i) = std::env::var("STREAM_DOCK_HA_SHOT_TILE").ok().and_then(|v| v.parse().ok()) {
                self.selected_tile = Some(i);
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_SETTINGS").is_some() {
                self.tab = Tab::Settings;
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_LOOK").is_some() {
                self.tab = Tab::Look;
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_MONITOR").is_some() {
                self.tab = Tab::Monitor;
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_ADJUST").is_some() {
                self.tab = Tab::Monitor;
                self.adjusting = true;
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_BROWSER").is_some() {
                self.browser = Some(EntityBrowser::new(BrowserTarget::NewKeys));
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_SMALL").is_some() {
                small_pad::start(self.shared.clone());
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_CAMERA").is_some() {
                // A made-up picture, for checking how a camera key looks.
                crate::camera::store("camera.demo", image::RgbImage::from_fn(640, 360, |x, y| {
                    let ring = ((x as i32 - 320).pow(2) + (y as i32 - 180).pow(2) - 22000).abs() < 3000;
                    if ring { image::Rgb([255, 220, 60]) } else { image::Rgb([(x / 3) as u8, (y * 2 / 3) as u8, 110]) }
                }));
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_PATTERN").is_some() {
                self.shared.alignment_pattern.store(true, Ordering::SeqCst);
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_ADD_DASHBOARD").is_some() {
                let shared = self.shared.clone();
                self.start_new_virtual_pad(&lock(&shared.cfg));
            }
            // The window changing the first virtual pad, at "rows,cols" if given.
            if let Some(size) = std::env::var_os("STREAM_DOCK_HA_SHOT_EDIT_VIRTUAL") {
                let shared = self.shared.clone();
                let cfg = lock(&shared.cfg);
                if let Some(pad_id) = cfg.pads.iter().find(|entry| entry.kind == PadKind::Dashboard).map(|entry| entry.id.clone()) {
                    self.edit_virtual_pad(&cfg, &pad_id);
                    let size = size.to_string_lossy().to_string();
                    if let (Some(editor), Some((rows, cols))) = (&mut self.pad_editor, size.split_once(',')) {
                        editor.rows = rows.trim().parse().unwrap_or(editor.rows);
                        editor.cols = cols.trim().parse().unwrap_or(editor.cols);
                    }
                }
            }
            // The question before deleting the first virtual pad.
            if std::env::var_os("STREAM_DOCK_HA_SHOT_DELETE_VIRTUAL").is_some() {
                let shared = self.shared.clone();
                let cfg = lock(&shared.cfg);
                self.confirm = cfg.pads.iter().find(|entry| entry.kind == PadKind::Dashboard).map(|entry| Confirm::RemovePad(entry.id.clone()));
            }
            if std::env::var_os("STREAM_DOCK_HA_SHOT_OPEN_HA").is_some() {
                self.tab = Tab::Settings;
                self.open_home_assistant = true;
            }
        }
        ctx.request_repaint();
        if self.frames == 120 {
            ctx.request_screenshot(move |image| {
                let code = match save_png(&path, &image) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("Couldn't save the picture: {e}");
                        1
                    }
                };
                if std::env::var_os("STREAM_DOCK_HA_SHOT_SMALL").is_some() {
                    small_pad::wait_for_pad();
                    small_pad::finish();
                }
                std::process::exit(code);
            });
        }
    }
}

/// The shortcuts in use, to notice when they change.
fn hotkey_list(cfg: &Config) -> Vec<String> {
    let mut list = vec![cfg.main_index().to_string()];
    for page in &cfg.main_profile().pages {
        for key in &page.keys {
            list.push(key.hotkey.clone());
        }
    }
    list
}

/// Stops what the window started: recording a shortcut and the alignment picture.
impl Drop for DockApp {
    fn drop(&mut self) {
        recorder::stop();
        self.shared.alignment_pattern.store(false, Ordering::SeqCst);
        crate::pad::wake();
    }
}

/// For tests: pointer and key events from `STREAM_DOCK_HA_SHOT_INPUT`, one
/// `frame what x y` to a line part, separated by `;`. Nothing touches the real
/// mouse or keyboard, and `--screenshot` saves no settings.
#[cfg(feature = "shots")]
fn scripted_events(frame: u32) -> Vec<egui::Event> {
    let Ok(script) = std::env::var("STREAM_DOCK_HA_SHOT_INPUT") else { return Vec::new() };
    let mut events = Vec::new();
    for step in script.split(';') {
        let parts: Vec<&str> = step.split_whitespace().collect();
        let at_frame = parts.first().and_then(|at| at.parse::<u32>().ok());
        // Ctrl is let go the frame after, so the key press sees it held.
        if parts.get(1) == Some(&"ctrl") && at_frame.map(|at| at + 1) == Some(frame) {
            events.push(egui::Event::ModifiersChanged(egui::Modifiers::default()));
        }
        if at_frame != Some(frame) {
            continue;
        }
        let number = |i: usize| parts.get(i).and_then(|n| n.parse::<f32>().ok()).unwrap_or(0.0);
        let at = egui::pos2(number(2), number(3));
        let modifiers = egui::Modifiers::default();
        let button = |button, pressed| egui::Event::PointerButton { pos: at, button, pressed, modifiers };
        match parts.get(1).copied().unwrap_or_default() {
            "move" => events.push(egui::Event::PointerMoved(at)),
            // `frame scroll x y by`: the wheel turned over a place, by points down.
            "scroll" => {
                events.push(egui::Event::PointerMoved(at));
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -number(4)),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers,
                });
            }
            "press" => {
                events.push(egui::Event::PointerMoved(at));
                events.push(button(egui::PointerButton::Primary, true));
            }
            "release" => {
                events.push(egui::Event::PointerMoved(at));
                events.push(button(egui::PointerButton::Primary, false));
            }
            "rpress" => {
                events.push(egui::Event::PointerMoved(at));
                events.push(button(egui::PointerButton::Secondary, true));
            }
            "rrelease" => {
                events.push(egui::Event::PointerMoved(at));
                events.push(button(egui::PointerButton::Secondary, false));
            }
            "key" => {
                if let Some(key) = parts.get(2).and_then(|name| egui::Key::from_name(name)) {
                    for pressed in [true, false] {
                        events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
                    }
                }
            }
            "ctrl" => {
                let key = parts.get(2).and_then(|name| egui::Key::from_name(name));
                if let Some(key) = key {
                    let modifiers = egui::Modifiers::COMMAND;
                    events.push(egui::Event::ModifiersChanged(modifiers));
                    for pressed in [true, false] {
                        events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
                    }
                }
            }
            _ => {}
        }
    }
    events
}

impl eframe::App for DockApp {
    #[cfg(feature = "shots")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if self.shot.is_some() {
            if std::env::var_os("STREAM_DOCK_HA_SHOT_INPUT").is_some() {
                // The real mouse must not steer a scripted test.
                raw_input.events.retain(|event| {
                    !matches!(event, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::PointerGone)
                });
            }
            raw_input.events.extend(scripted_events(self.frames));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let shared = self.shared.clone();
        let mut guard = lock(&shared.cfg);
        let cfg = &mut *guard;

        self.frames += 1;
        // A test copy can open its first pad on screen in a window of its
        // own, to see it as Windows draws it.
        if self.frames == 1 && crate::model::sandbox_folder().is_some() && std::env::var_os("STREAM_DOCK_HA_TEST_ON_SCREEN").is_some() {
            if let Some(pad_id) = cfg.pads_in_use().next().map(|entry| entry.id.clone()) {
                self.toggle_on_screen(&pad_id);
            }
        }
        let main_visible = ctx.input(|input| input.viewport().visible()).unwrap_or(true);
        // Pads on screen asked for from the tray.
        for pad_id in std::mem::take(&mut *lock(&shared.screen_requests)) {
            if cfg.pad_profile_of(&pad_id).is_some() {
                self.toggle_on_screen(&pad_id);
            }
        }
        // Closing the window while pads are on screen only hides it: they are
        // drawn by this window, and closing it under them used to freeze the app.
        if ctx.input(|input| input.viewport().close_requested()) && !self.on_screen.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        if cfg.theme != self.theme {
            self.theme = cfg.theme;
            apply_theme(&ctx, cfg.theme);
            self.drawn.clear();
        }
        LIGHT.store(ctx.theme() == Theme::Light, Ordering::Relaxed);
        if self.adjusting && !shared.alignment_pattern.load(Ordering::SeqCst) && self.frames > 2 && self.shot.is_none() {
            self.adjusting = false;
        }
        // A window opened from the icon should arrive in front of everything.
        // Once the pad on screen is up, the window started for it hides.
        if self.started_hidden && self.frames == 2 {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        if self.started_hidden && self.frames == 3 {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(self.home));
        }
        if self.frames == 3 && self.shot.is_none() && main_visible && !self.started_hidden {
            crate::win::bring_to_front(shared.window.load(Ordering::SeqCst) as _);
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        self.apply_picked(cfg);
        self.capture_recording(&ctx, cfg);
        self.key_shortcuts(&ctx, cfg);
        if self.selected.is_some_and(|i| i >= cfg.profile().key_count()) {
            self.selected = None;
        }

        // While it's hidden, only the pads on screen need drawing: they repaint
        // up to 30 times a second with Mini Monitor on.
        if main_visible || self.shot.is_some() {
            let bar = |fill| Frame::new().fill(fill).inner_margin(Margin::symmetric(14, 6));
            Panel::top("top")
                .exact_size(52.0)
                .show_separator_line(false)
                .frame(bar(c_panel()))
                .show(ui, |ui| self.top_bar(ui, cfg));
            Panel::bottom("status")
                .exact_size(30.0)
                .show_separator_line(false)
                .frame(bar(c_panel()))
                .show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        let status = lock(&shared.status).clone();
                        if status.is_empty() {
                            ui.label(dim("Ready").size(12.5));
                        } else {
                            ui.label(dim(status).size(12.5));
                        }
                        // The corner speaks only when something needs doing.
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let ha = lock(&shared.ha.status).clone();
                            if matches!(ha, ha::HaStatus::TokenRefused | ha::HaStatus::Failed(_)) {
                                let link = ui.link(RichText::new(ha.label()).color(c_warn()).size(12.5));
                                if link.on_hover_text("Open the Home Assistant settings").clicked() {
                                    self.tab = Tab::Settings;
                                    self.open_home_assistant = true;
                                }
                                ui.add_space(12.0);
                            }
                            let pad_problem = match cfg.profile().pad {
                                PadKind::Main => {
                                    let pad = lock(&shared.pad_status).clone();
                                    (!pad.is_working()).then(|| pad.label())
                                }
                                PadKind::Akp03e => (!small_pad::is_connected())
                                    .then(|| format!("{}: {}", PadKind::Akp03e.label(), small_pad::status())),
                                PadKind::Dashboard => None,
                            };
                            if let Some(text) = pad_problem {
                                ui.label(RichText::new(text).color(c_warn()).size(12.5));
                            }
                        });
                    });
                });
            Panel::right("side")
                .exact_size(316.0)
                .show_separator_line(false)
                .frame(Frame::new().fill(c_panel()).inner_margin(Margin::same(12)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.selectable_value(&mut self.tab, Tab::Actions, "Actions");
                        ui.selectable_value(&mut self.tab, Tab::Look, "Look");
                        ui.selectable_value(&mut self.tab, Tab::Monitor, "Monitor");
                        ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
                    });
                    ui.add_space(4.0);
                    match self.tab {
                        Tab::Actions => self.library(ui, cfg),
                        Tab::Look => {
                            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.look_settings(ui, cfg));
                        }
                        Tab::Monitor => {
                            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.monitor_settings(ui, cfg));
                        }
                        Tab::Settings => {
                            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.settings(ui, cfg));
                        }
                    }
                });
            // Taller when a key needs more room: drag its top edge.
            Panel::bottom("inspector")
                .resizable(true)
                .default_size(250.0)
                .size_range(180.0..=520.0)
                .show_separator_line(false)
                .frame(
                    Frame::new()
                        .fill(c_panel())
                        .inner_margin(Margin::same(12))
                        .stroke(Stroke::new(1.0, c_line())),
                )
                .show(ui, |ui| {
                    // A panel that overflows gives up space at its top, which lets the
                    // pad grow over it, so what's inside gets a fixed area to work in.
                    let area = ui.available_rect_before_wrap();
                    let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)));
                    inside.shrink_clip_rect(area);
                    self.inspector(&mut inside, cfg);
                    ui.allocate_rect(area, Sense::hover());
                });
            CentralPanel::default()
                .frame(Frame::new().fill(c_bg()).inner_margin(Margin::same(12)))
                .show(ui, |ui| {
                    if self.show_hint {
                        Frame::new()
                            .fill(ACCENT.gamma_multiply(0.22))
                            .corner_radius(8)
                            .inner_margin(Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Closing this window keeps the app running. Find it by the clock, bottom right, or under the ^ arrow.");
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.button("Got it").clicked() {
                                            self.show_hint = false;
                                        }
                                    });
                                });
                            });
                    }
                    self.pad_offer(ui, cfg);
                    self.page_tabs(ui, cfg);
                    self.grid(ui, cfg);
                });

            self.pad_editor_dialog(&ctx, cfg);
            self.confirm_dialog(&ctx, cfg);
            self.startup_dialog(&ctx);
            self.entity_browser(&ctx, cfg);
            self.drag_preview(&ctx);
        }
        self.track_changes(&ctx, cfg);
        self.virtual_pads(&ctx, cfg);
        // A hidden window with no pad left on screen has nothing to show.
        if !main_visible && self.on_screen.is_empty() && self.frames > 2 && self.shot.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        drop(guard);
        self.run_pending_presses();
        if self.quitting {
            tray::quit();
        }
        #[cfg(feature = "shots")]
        self.take_picture_when_ready(&ctx);
    }

    /// Clear, so a pad on screen shows only the pad. The main window paints
    /// its own background everywhere.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests;

/// For tests elsewhere: drops the library's item that makes `binding` onto
/// a key, as dragging it there does.
#[cfg(test)]
pub fn add_library_action_for_tests(key: &mut DockKey, binding: crate::model::HaAction) {
    let wanted = Action::HomeAssistant(binding);
    let index = library::LIBRARY.iter().position(|item| (item.make)() == wanted).expect("no such library item");
    library::add_library_action(key, index);
}
