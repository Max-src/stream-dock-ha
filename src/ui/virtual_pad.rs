//! Pads on screen: a borderless window that is just the pad, where clicking a
//! key, button or knob does what pressing it on the real pad does, and Mini
//! Monitor shows as it does on the pad. Drag the plastic to move it and an edge
//! to resize it. It works with or without the pad plugged in, so a pad can be
//! prepared and tried in advance.

use super::*;

/// What "Mouse on Mini Monitor" does, in the pad's menu and in the settings.
pub(super) const MOUSE_HINT: &str = "Over the picture, your mouse pointer is on the display, as on a real monitor: hover, click, double-click, drag windows, use menus, the wheel and the keyboard there. Moving off the picture brings it back. Mini Monitor stays on; the pad's other buttons and knobs, and the real pad's keys, still leave it.";

/// How close to a window's edge a drag resizes it rather than moving it.
const EDGE: f32 = 7.0;

/// Which edge or corner of a window a point is on, if any.
fn edge_at(area: Rect, at: egui::Pos2) -> Option<egui::ResizeDirection> {
    use egui::ResizeDirection::*;
    let (west, east) = (at.x < area.min.x + EDGE, at.x > area.max.x - EDGE);
    let (north, south) = (at.y < area.min.y + EDGE, at.y > area.max.y - EDGE);
    match (north, south, west, east) {
        (true, _, true, _) => Some(NorthWest),
        (true, _, _, true) => Some(NorthEast),
        (_, true, true, _) => Some(SouthWest),
        (_, true, _, true) => Some(SouthEast),
        (true, _, _, _) => Some(North),
        (_, true, _, _) => Some(South),
        (_, _, true, _) => Some(West),
        (_, _, _, true) => Some(East),
        _ => None,
    }
}

fn resize_cursor(direction: egui::ResizeDirection) -> egui::CursorIcon {
    use egui::ResizeDirection::*;
    match direction {
        North | South => egui::CursorIcon::ResizeVertical,
        East | West => egui::CursorIcon::ResizeHorizontal,
        NorthWest | SouthEast => egui::CursorIcon::ResizeNwSe,
        NorthEast | SouthWest => egui::CursorIcon::ResizeNeSw,
    }
}

/// This app's own window with this title, once Windows has made it.
fn own_window(title: &str) -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};
    let name = crate::win::wide(title);
    unsafe {
        let window = FindWindowW(std::ptr::null(), name.as_ptr());
        if window.is_null() {
            return None;
        }
        let mut process = 0;
        GetWindowThreadProcessId(window, &mut process);
        (process == GetCurrentProcessId()).then_some(window)
    }
}

/// Windows 11 draws a frameless window's corners, a thin border and a shadow
/// round it, which showed round the pad, most plainly round a see-through one
/// as a frame and a line along its top. Windows draws nothing round it here:
/// the pad paints its own rounded corners, and only its own pixels show.
fn clean_corners(window: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_DONOTROUND,
    };
    unsafe {
        let disabled: u32 = DWMNCRP_DISABLED as u32;
        DwmSetWindowAttribute(window, DWMWA_NCRENDERING_POLICY as _, (&disabled as *const u32).cast(), 4);
        let square: u32 = DWMWCP_DONOTROUND as u32;
        DwmSetWindowAttribute(window, DWMWA_WINDOW_CORNER_PREFERENCE as _, (&square as *const u32).cast(), 4);
        // DWMWA_COLOR_NONE: no border at all, on versions that still draw one.
        let no_border: u32 = 0xFFFF_FFFE;
        DwmSetWindowAttribute(window, DWMWA_BORDER_COLOR as _, (&no_border as *const u32).cast(), 4);
    }
}

/// egui asks winit for a shadow on every window without decorations, and
/// winit then leaves the window's top row of pixels out of what the pad
/// draws. Windows painted that row: a thin line along the pad's top, in the
/// accent colour once the pad was clicked. The pad's windows take the whole
/// window for themselves instead, before winit sees the message.
fn take_whole_window(window: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
    };
    unsafe {
        SetWindowSubclass(window, Some(whole_window), 1, 0);
        let keep = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
        SetWindowPos(window, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | keep);
    }
}

unsafe extern "system" fn whole_window(
    window: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    _id: usize,
    _data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_NCCALCSIZE;
    // The window's whole rectangle is the pad's: no frame, not even one row.
    if message == WM_NCCALCSIZE && wparam != 0 {
        return 0;
    }
    windows_sys::Win32::UI::Shell::DefSubclassProc(window, message, wparam, lparam)
}

/// winit keeps a title bar's styles on a window without decorations and
/// only stretches its own drawing over them, so Windows 11 still drew a thin
/// line of frame along the pad's top. With those styles gone there's no frame
/// to draw. winit puts them back when it changes the window, for instance its
/// level, so they're looked at every frame and taken off again.
fn drop_frame(window: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_EX_WINDOWEDGE, WS_POPUP, WS_THICKFRAME,
    };
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_STYLE) as u32;
        let frame = WS_CAPTION | WS_THICKFRAME;
        if style & frame == 0 {
            return;
        }
        SetWindowLongPtrW(window, GWL_STYLE, ((style & !frame) | WS_POPUP) as isize);
        let extended = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(window, GWL_EXSTYLE, (extended & !WS_EX_WINDOWEDGE) as isize);
        let keep = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
        SetWindowPos(window, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | keep);
    }
}

/// A resize of a pad on screen under way, done here rather than by Windows: a
/// window Windows can resize keeps a thin frame, which showed along the top.
#[derive(Clone)]
pub(super) struct PadResize {
    /// The pad's id.
    pub(super) pad_id: String,
    pub(super) direction: egui::ResizeDirection,
    /// Where the pointer started, on the screen.
    pub(super) from: egui::Pos2,
    /// The window at the start, on the screen.
    pub(super) start: Rect,
    pub(super) tile: f32,
}

/// The size and place a resize from this edge or corner gives, keeping the
/// opposite side where it is and the pad's own shape.
pub(super) fn resized(resize: &PadResize, now: egui::Pos2, size_of: impl Fn(f32) -> Vec2) -> Rect {
    use egui::ResizeDirection::*;
    let moved = now - resize.from;
    let (start, direction) = (resize.start, resize.direction);
    let wider = match direction {
        East | NorthEast | SouthEast => moved.x,
        West | NorthWest | SouthWest => -moved.x,
        _ => 0.0,
    };
    let taller = match direction {
        South | SouthEast | SouthWest => moved.y,
        North | NorthEast | NorthWest => -moved.y,
        _ => 0.0,
    };
    let across = (start.width() + wider) / start.width();
    let down = (start.height() + taller) / start.height();
    let scale = match direction {
        East | West => across,
        North | South => down,
        _ => across.max(down),
    };
    let size = size_of((resize.tile * scale).clamp(36.0, 260.0));
    let left = if matches!(direction, West | NorthWest | SouthWest) { start.max.x - size.x } else { start.min.x };
    let top = if matches!(direction, North | NorthEast | NorthWest) { start.max.y - size.y } else { start.min.y };
    Rect::from_min_size(egui::pos2(left, top), size)
}

/// What a click in a pad on screen's menu asked for.
#[derive(Clone, Copy, PartialEq)]
enum PadRequest {
    Nothing,
    Close,
    ToggleOnTop,
    ToggleSeamless,
    ToggleMouse,
    /// Open the main window on this pad's running profile and page.
    Configure,
}

/// A pad on screen's window title. Pad names are unique, so each window has its own.
fn title_of(name: &str) -> String {
    format!("{name} on screen")
}

/// Where a Mini Monitor tile shows on a pad on screen.
fn tile_rect(layout: &PadLayout, kind: PadKind, tile: &crate::monitor::Tile) -> Rect {
    match kind {
        PadKind::Main if tile.is_band() => layout.band_tile(tile.row),
        // A virtual pad never shows Mini Monitor; it would be cut as the 293S's keys.
        PadKind::Main | PadKind::Dashboard => layout.home(tile.row * 5 + tile.column),
        PadKind::Akp03e => layout.home(tile.row * crate::model::SMALL_COLS + tile.column),
    }
}

/// Where Mini Monitor's picture is drawn on a pad on screen: each piece's place
/// on the pad and the part of the shared picture it shows, in its pixels.
/// Across the gaps it's one piece, in the display's own proportions.
fn picture_pieces(layout: &PadLayout, kind: PadKind, shown: &crate::monitor::Layout, seamless: bool) -> Vec<(Rect, Rect)> {
    let (width, height) = (shown.width as f32, shown.height as f32);
    if seamless {
        let whole = shown.tiles.iter().map(|tile| tile_rect(layout, kind, tile)).reduce(|a, b| a.union(b));
        return whole
            .map(|whole| {
                // Kept in the display's own proportions, so a circle stays round.
                let scale = (whole.width() / width).min(whole.height() / height);
                (Rect::from_center_size(whole.center(), vec2(width, height) * scale), Rect::from_min_size(egui::Pos2::ZERO, vec2(width, height)))
            })
            .into_iter()
            .collect();
    }
    shown
        .tiles
        .iter()
        .map(|tile| {
            let part = Rect::from_min_size(egui::pos2(tile.x as f32, tile.y as f32), vec2(tile.width as f32, tile.height as f32));
            (tile_rect(layout, kind, tile), part)
        })
        .collect()
}

/// Where Mini Monitor's pieces are on the desktop, in pixels, for the mouse
/// hook. The edges stay the window's, for resizing.
fn mouse_zones(title: &str, pieces: &[(Rect, Rect)], inside: Rect, scale: f32, size: (i32, i32), from: crate::monitor::Area) -> Vec<crate::screen_mouse::Zone> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    let Some(window) = self::own_window(title) else { return Vec::new() };
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(window, &mut origin) } == 0 {
        return Vec::new();
    }
    let corner = |at: egui::Pos2| (origin.x as f32 + at.x * scale, origin.y as f32 + at.y * scale);
    pieces
        .iter()
        .filter_map(|(place, part)| {
            let shown = place.intersect(inside);
            if !shown.is_positive() {
                return None;
            }
            // The part of the picture that the clipped piece still shows.
            let along = |at: egui::Pos2| part.min + (at - place.min) / place.size() * part.size();
            let (start, end) = (corner(shown.min), corner(shown.max));
            let (first, last) = (along(shown.min), along(shown.max));
            Some(crate::screen_mouse::Zone {
                window: window as isize,
                place: [start.0, start.1, end.0, end.1],
                part: [first.x, first.y, last.x, last.y],
                size,
                from,
            })
        })
        .collect()
}

/// The point of the shared picture under a point of the pad on screen.
fn picture_point(pieces: &[(Rect, Rect)], at: egui::Pos2) -> Option<egui::Pos2> {
    let (place, part) = pieces.iter().find(|(place, _)| place.contains(at))?;
    let along = (at - place.min) / place.size();
    Some(part.min + along * part.size())
}

impl DockApp {
    /// Opens or closes a pad on screen.
    pub(super) fn toggle_on_screen(&mut self, pad_id: &str) {
        if let Some(at) = self.on_screen.iter().position(|open| open == pad_id) {
            self.on_screen.remove(at);
        } else {
            self.on_screen.push(pad_id.to_string());
        }
    }

    /// Each pad on screen, in a borderless window of its own that fits the pad.
    pub(super) fn virtual_pads(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        // A pad whose last profile was deleted isn't on screen any more.
        self.on_screen.retain(|pad_id| cfg.pad_profile_of(pad_id).is_some());
        // Mini Monitor needs to know which pads are on screen, even without the real pad.
        lock(&self.shared.on_screen).clone_from(&self.on_screen);
        self.clean_cornered.retain(|pad_id| self.on_screen.contains(pad_id));
        self.mouse_zones.clear();
        let level = if cfg.on_screen_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
        for pad_id in self.on_screen.clone() {
            let Some(index) = cfg.pad_profile_of(&pad_id) else { continue };
            let Some((kind, name)) = cfg.pad(&pad_id).map(|entry| (entry.kind, entry.name.clone())) else { continue };
            let (rows, cols) = (cfg.profiles[index].rows, cfg.profiles[index].cols);
            let start = PadLayout::scaled(kind, rows, cols, 80.0, egui::Pos2::ZERO).device.size();
            let smallest = PadLayout::scaled(kind, rows, cols, 36.0, egui::Pos2::ZERO).device.size();
            let window = egui::ViewportBuilder::default()
                .with_title(title_of(&name))
                .with_window_level(level)
                .with_decorations(false)
                .with_transparent(true)
                // Resized by the pad itself: a window Windows can resize keeps a thin frame.
                .with_resizable(false)
                .with_inner_size(start)
                .with_min_inner_size(smallest);
            let id = egui::ViewportId::from_hash_of(("pad-on-screen", &pad_id));
            let shown: &Config = cfg;
            let request = ctx.show_viewport_immediate(id, window, |ui, _| {
                let request = self.virtual_pad(ui, shown, &pad_id, rows, cols);
                if ui.ctx().input(|input| input.viewport().close_requested()) {
                    PadRequest::Close
                } else {
                    request
                }
            });
            match request {
                PadRequest::Close => self.on_screen.retain(|open| *open != pad_id),
                PadRequest::ToggleOnTop => {
                    cfg.on_screen_on_top = !cfg.on_screen_on_top;
                    let level = if cfg.on_screen_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
                    for open in &self.on_screen {
                        let id = egui::ViewportId::from_hash_of(("pad-on-screen", open));
                        ctx.send_viewport_cmd_to(id, ViewportCommand::WindowLevel(level));
                    }
                }
                PadRequest::ToggleSeamless => cfg.on_screen_seamless = !cfg.on_screen_seamless,
                PadRequest::ToggleMouse => cfg.on_screen_mouse = !cfg.on_screen_mouse,
                PadRequest::Configure => {
                    cfg.show_pad_id(&pad_id);
                    self.selected = None;
                    self.selected_tile = None;
                    self.tab = Tab::Actions;
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Focus);
                    crate::win::bring_to_front(self.shared.window.load(Ordering::SeqCst) as _);
                }
                PadRequest::Nothing => {}
            }
            // Once its window exists: clean corners, and in front when it opens.
            if !ctx.embed_viewports() {
                if let Some(window) = own_window(&title_of(&name)) {
                    drop_frame(window);
                }
            }
            if !ctx.embed_viewports() && !self.clean_cornered.contains(&pad_id) {
                if let Some(window) = own_window(&title_of(&name)) {
                    clean_corners(window);
                    take_whole_window(window);
                    if !cfg.on_screen_on_top {
                        crate::win::bring_to_front(window);
                    }
                    self.clean_cornered.push(pad_id.clone());
                }
            }
        }
        if !self.shared.read_only {
            crate::screen_mouse::set_zones(std::mem::take(&mut self.mouse_zones));
        }
    }

    /// One pad as it is now: its running profile and page, its band, Mini
    /// Monitor, and what was last pressed. Clicks are run once this frame lets
    /// go of the settings. Says what its menu asked for.
    fn virtual_pad(&mut self, ui: &mut Ui, cfg: &Config, pad_id: &str, rows: usize, cols: usize) -> PadRequest {
        let Some(index) = cfg.pad_profile_of(pad_id) else { return PadRequest::Close };
        let Some(entry) = cfg.pad(pad_id) else { return PadRequest::Close };
        let kind = entry.kind;
        let profile = &cfg.profiles[index];
        let page = profile.active_page;
        let ctx = ui.ctx().clone();
        let area = ui.max_rect();
        let layout = PadLayout::fitted(kind, rows, cols, area, (30.0, 260.0));
        // The window keeps the pad's own shape, so there's nothing around it.
        // Drawn inside another window (in test pictures), it can't move or resize its own.
        let own_window = !ctx.embed_viewports();
        let held = ctx.input(|input| input.pointer.any_down());
        if own_window && !held && (layout.device.size() - area.size()).length() > 0.5 {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(layout.device.size()));
        }
        // The plastic fills the whole window, so no sliver of anything else
        // can show at an edge while the size settles. No outline: the real pad
        // has none. A see-through front isn't painted at all, so only the keys
        // show; the window still takes clicks there, to move the pad.
        let face = entry.front().unwrap_or([0, 0, 0]);
        let painter = ui.painter().clone();
        if entry.front().is_some() {
            painter.rect_filled(area, layout.corner(), Color32::from_rgb(face[0], face[1], face[2]));
        }

        let response = ui.interact(area, Id::NULL.with(("pad-on-screen", pad_id)), Sense::click_and_drag());
        let pointer = response.hover_pos();
        // A control only where it really is: the gaps between keys are plastic.
        let hit = |at: egui::Pos2| {
            layout.cell_at(at, rows).filter(|cell| {
                (kind == PadKind::Akp03e && small_knob_of_cell(*cell).is_some()) || layout.home(*cell).contains(at)
            })
        };
        let under = pointer.and_then(hit).map(|cell| profile.owner_of(page, cell));
        let edge = pointer.and_then(|at| edge_at(area, at));
        let shared = self.shared.clone();
        let picture_on = cfg.monitor_on(kind) || (cfg.monitor_pad == kind && shared.alignment_pattern.load(Ordering::SeqCst));
        let shown = if picture_on { crate::monitor::shown(kind) } else { None };
        let pieces = shown.as_ref().map_or(Vec::new(), |shown| picture_pieces(&layout, kind, &shown.layout, cfg.on_screen_seamless));
        // With the mouse on Mini Monitor, the picture acts as the display does.
        // Its presses are taken before any window sees them (screen_mouse), so
        // menus open on the display stay open; this window only says where it is.
        let mouse_on = cfg.on_screen_mouse && cfg.monitor_on(kind);
        if let Some(from) = shown.as_ref().filter(|_| mouse_on && own_window && !shared.read_only).and_then(|shown| shown.from) {
            let size = shown.as_ref().map_or((0, 0), |shown| (shown.layout.width, shown.layout.height));
            self.mouse_zones.extend(mouse_zones(&title_of(&entry.name), &pieces, area.shrink(EDGE), ctx.pixels_per_point(), size, from));
        }
        let display_at = |at: egui::Pos2| {
            let shown = shown.as_ref().filter(|_| cfg.on_screen_mouse && cfg.monitor_on(kind))?;
            picture_point(&pieces, at).and_then(|point| shown.desktop_point(point.x, point.y))
        };
        let on_display = pointer.filter(|at| edge_at(area, *at).is_none()).and_then(display_at);
        if let Some(direction) = edge {
            ctx.set_cursor_icon(resize_cursor(direction));
        } else if on_display.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        } else if under.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // A press on an edge resizes, a press on the plastic moves the pad.
        let window = ctx.input(|input| input.viewport().inner_rect);
        if own_window && response.hovered() && ctx.input(|input| input.pointer.primary_pressed()) {
            match (edge, window, pointer) {
                (Some(direction), Some(start), Some(at)) => {
                    let from = start.min + at.to_vec2();
                    self.pad_resize = Some(PadResize { pad_id: pad_id.to_string(), direction, from, start, tile: layout.tile });
                }
                (None, _, _) if under.is_none() && on_display.is_none() => ctx.send_viewport_cmd(ViewportCommand::StartDrag),
                _ => {}
            }
        }
        // The wheel over the picture scrolls whatever is there on the display.
        if let Some(at) = on_display.filter(|_| response.hovered()) {
            let notches: f32 = ctx.input(|input| {
                input.events.iter().map(|event| match event {
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. } => delta.y,
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } => delta.y / 50.0,
                    egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Page, delta, .. } => delta.y * 3.0,
                    _ => 0.0,
                }).sum()
            });
            if notches != 0.0 {
                self.display_mouse(at, crate::monitor::DisplayMouse::Wheel((notches * 120.0).round() as i32));
            }
        }
        if let Some(resize) = self.pad_resize.clone().filter(|resize| resize.pad_id == pad_id) {
            let now = ctx.input(|input| input.pointer.latest_pos()).zip(window).map(|(at, window)| window.min + at.to_vec2());
            if !held {
                self.pad_resize = None;
            } else if let Some(now) = now {
                let wanted = resized(&resize, now, |tile| PadLayout::scaled(kind, rows, cols, tile, egui::Pos2::ZERO).device.size());
                if window.is_some_and(|window| (window.min - wanted.min).length() > 0.5) {
                    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(wanted.min));
                }
                if (wanted.size() - area.size()).length() > 0.5 {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(wanted.size()));
                }
                ctx.set_cursor_icon(resize_cursor(resize.direction));
                ctx.request_repaint();
            }
        }

        let pressed = |cell: usize| actions::just_pressed(&shared, kind, pad_id, cell);
        let style = profile.style_on_pad(&cfg.style);
        let entities = lock(&shared.ha.entities);
        let keys = &profile.pages[page].keys;
        let showing = frames::Showing { profile: index, page, outside: face };
        let frame = frames::pad_frame_showing(&shared, cfg, &entities, pad_id, showing).unwrap_or_default();
        let mut lit = false;
        let mut band_covered = false;
        if picture_on {
            // What the real pad shows in Mini Monitor, tile by tile.
            let slot = kind == PadKind::Akp03e;
            match &shown {
                Some(shown) => {
                    let fresh = self.monitor_textures.get(&slot).is_some_and(|(version, _)| *version == shown.version);
                    if !fresh {
                        let size = [shown.layout.width as usize, shown.layout.height as usize];
                        let pixels = egui::ColorImage::from_rgb(size, &shown.canvas);
                        let texture = ctx.load_texture(format!("mini-monitor-{}", kind.label()), pixels, TextureOptions::LINEAR);
                        self.monitor_textures.insert(slot, (shown.version, texture));
                    }
                    let texture = self.monitor_textures[&slot].1.id();
                    let size = vec2(shown.layout.width as f32, shown.layout.height as f32);
                    band_covered = shown.layout.tiles.iter().any(|tile| tile.is_band());
                    for (place, part) in &pieces {
                        let uv = Rect::from_min_max((part.min.to_vec2() / size).to_pos2(), (part.max.to_vec2() / size).to_pos2());
                        // Across the gaps it's square-edged, like a small monitor.
                        let corners = if cfg.on_screen_seamless { CornerRadius::ZERO } else { key_radius(*place, &style) };
                        painter.add(egui::epaint::RectShape::filled(*place, corners, Color32::WHITE).with_texture(texture, uv));
                    }
                }
                None => {
                    for cell in (0..rows * cols).filter(|cell| kind.cell_has_screen(*cell)) {
                        painter.rect_filled(layout.home(cell), key_radius(layout.home(cell), &style), Color32::BLACK);
                    }
                    painter.text(layout.device.center(), Align2::CENTER_CENTER, "Mini Monitor is starting…", FontId::proportional(13.0), c_dim());
                }
            }
            // New pictures ask for a repaint themselves; this is only a fallback.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            for cell in (0..rows * cols).filter(|cell| kind.cell_has_screen(*cell)) {
                let owner = profile.owner_of(page, cell);
                let look = frame.look_of(cell).cloned();
                let state = (false, under == Some(owner), false);
                self.paint_look(&painter, layout.home(cell), look, &keys[owner], &style, state);
                if pressed(owner) {
                    painter.rect_filled(layout.home(cell), 12, Color32::from_white_alpha(70));
                    lit = true;
                }
            }
        }
        if crate::screen_off::pad_asleep(cfg, pad_id) {
            for cell in (0..rows * cols).filter(|cell| kind.cell_has_screen(*cell)) {
                painter.rect_filled(layout.home(cell), key_radius(layout.home(cell), &style), Color32::BLACK);
            }
        }
        if kind.has_band() {
            painter.rect_filled(layout.band_strip(), band_radius(layout.band_tile(0), &frame.band_style), Color32::BLACK);
        }
        if let Some(plan) = &layout.plan {
            let state = |cell: usize| {
                lit |= pressed(cell);
                part_color((false, under == Some(cell), false, pressed(cell)))
            };
            paint_small_controls(&painter, plan, keys, state, on_face(face), |_| true);
        }
        if kind.has_band() && !band_covered && !crate::screen_off::pad_asleep(cfg, pad_id) {
            for (tile, shown) in frame.band_faces().iter().enumerate() {
                self.paint_face(ui, layout.band_tile(tile), shown, &frame.band_style);
            }
        }
        drop(entities);
        // Where the click let go: by then egui no longer knows where it began.
        let at = response.interact_pointer_pos().or(pointer).filter(|at| edge_at(area, *at).is_none());
        // On Mini Monitor with the mouse on, a click is a click on the display
        // there, and Mini Monitor stays. Anywhere else it's a press, as on the pad.
        let clicked_display = at.and_then(display_at);
        if response.clicked() {
            match clicked_display {
                Some(point) => self.display_mouse(point, crate::monitor::DisplayMouse::Left),
                None => {
                    // The bars round the display are part of the picture too:
                    // with the mouse on it, they don't leave Mini Monitor.
                    let on_bars = mouse_on && at.and_then(|at| picture_point(&pieces, at)).is_some();
                    if let Some(cell) = at.and_then(hit).filter(|_| !on_bars) {
                        self.pending_presses.push((pad_id.to_string(), cell));
                    }
                }
            }
        }
        let right_on_display = response.secondary_clicked() && clicked_display.is_some();
        if let Some(point) = clicked_display.filter(|_| right_on_display) {
            self.display_mouse(point, crate::monitor::DisplayMouse::Right);
        }
        if let Some(point) = clicked_display.filter(|_| response.middle_clicked()) {
            self.display_mouse(point, crate::monitor::DisplayMouse::Middle);
        }
        let mut request = PadRequest::Nothing;
        // The pad's own menu is on the plastic; on the display a right-click is the display's.
        let menu = (!right_on_display).then_some(&response);
        if let Some(response) = menu {
            response.context_menu(|ui| {
            if ui.button("Configure this pad").on_hover_text("Opens the main window on this pad's profile and page").clicked() {
                request = PadRequest::Configure;
                ui.close();
            }
            let mut seamless = cfg.on_screen_seamless;
            if ui.checkbox(&mut seamless, "Mini Monitor across the gaps").on_hover_text("One picture over the keys and the gaps, with square edges, like a small monitor").clicked() {
                request = PadRequest::ToggleSeamless;
                ui.close();
            }
            let mut mouse = cfg.on_screen_mouse;
            if ui.checkbox(&mut mouse, "Mouse on Mini Monitor").on_hover_text(MOUSE_HINT).clicked() {
                request = PadRequest::ToggleMouse;
                ui.close();
            }
            let mut on_top = cfg.on_screen_on_top;
            if ui.checkbox(&mut on_top, "Keep on top of other windows").clicked() {
                request = PadRequest::ToggleOnTop;
                ui.close();
            }
            ui.separator();
            if ui.button("Close this pad").clicked() {
                request = PadRequest::Close;
                ui.close();
            }
            });
        }
        let later = if lit { 80 } else { 1000 };
        ctx.request_repaint_after(std::time::Duration::from_millis(later));
        request
    }

    /// A click or a turn of the wheel on the display, from a pad on screen.
    /// A test picture never acts on the PC, but says what it would do.
    fn display_mouse(&mut self, at: (i32, i32), what: crate::monitor::DisplayMouse) {
        if self.shared.read_only {
            self.shared.set_status(format!("{what:?} at {}, {} on the display (not done in a test)", at.0, at.1));
        } else {
            crate::monitor::mouse_on_display(at, what);
        }
    }

    /// Runs the presses made on pads on screen, as the real pads would.
    pub(super) fn run_pending_presses(&mut self) {
        for (pad_id, cell) in std::mem::take(&mut self.pending_presses) {
            if self.shared.read_only {
                // A test picture never acts on the PC, but shows the press.
                let Some((kind, name)) = lock(&self.shared.cfg).pad(&pad_id).map(|entry| (entry.kind, entry.name.clone())) else { continue };
                actions::mark_pressed(&self.shared, kind, &pad_id, cell);
                self.shared.set_status(format!("{name} cell {cell} pressed on screen (not run in a test)"));
            } else {
                actions::press_pad(&self.shared, &pad_id, cell, actions::PressSource::Screen);
            }
        }
    }
}
