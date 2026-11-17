//! Drawing the pads in the window: keys, the AKP03E's buttons and knobs, the band, and dragging.

use super::*;
use crate::model::layout::{band_height, BAND_TILE, BODY_CORNER, EDGE};

/// How quickly keys slide, per second. Higher is quicker.
pub(super) const GLIDE_SPEED: f32 = 16.0;

/// One step of a key's slide towards where it should be, slowing as it arrives.
pub(super) fn glide_toward(now: Vec2, wanted: Vec2, seconds: f32) -> Vec2 {
    let next = now + (wanted - now) * (1.0 - (-seconds * GLIDE_SPEED).exp());
    if (wanted - next).length() < 0.4 {
        wanted
    } else {
        next
    }
}

/// Height of the small captions under the AKP03E's buttons and knobs.
pub(super) const CAPTION: f32 = 17.0;

/// Where the AKP03E's parts sit: six keys with three round buttons under them,
/// one big knob at the top right and two small ones below it, as on the real
/// pad.
#[derive(Clone, Copy)]
pub(super) struct SmallPlan {
    pub(super) first: egui::Pos2,
    pub(super) tile: f32,
    pub(super) gap: f32,
    /// Lower left, top and lower right, like the cells' columns.
    pub(super) knobs: [(egui::Pos2, f32); 3],
    pub(super) buttons: [(egui::Pos2, f32); 3],
}

impl SmallPlan {
    const SPACER: f32 = 0.35;
    const KNOBS_WIDE: f32 = 2.35;
    const BIG: f32 = 0.8;
    const SMALL: f32 = 0.5;
    const BUTTON: f32 = 0.31;
    const INNER: f32 = 0.46;

    pub(super) fn inner_height(tile: f32, gap: f32) -> f32 {
        let keys = 2.0 * tile + gap + 0.26 * tile + 2.0 * Self::BUTTON * tile + CAPTION;
        let knobs = 2.0 * (Self::BIG + Self::SMALL) * tile + 0.1 * tile + 2.0 * CAPTION;
        keys.max(knobs)
    }

    pub(super) fn device_size(tile: f32, gap: f32, pad: f32) -> Vec2 {
        let wide = (3.0 + Self::SPACER + Self::KNOBS_WIDE) * tile + 2.0 * gap;
        vec2(wide + pad * 2.0, Self::inner_height(tile, gap) + pad * 2.0)
    }

    pub(super) fn new(device: Rect, tile: f32, gap: f32, pad: f32) -> SmallPlan {
        let first = device.min + vec2(pad, pad);
        let right = first.x + 3.0 * tile + 2.0 * gap + Self::SPACER * tile;
        let big = Self::BIG * tile;
        let small = Self::SMALL * tile;
        let top = egui::pos2(right + Self::KNOBS_WIDE * tile / 2.0, first.y + big);
        let lower_y = first.y + 2.0 * big + CAPTION + 0.1 * tile + small;
        let knobs = [
            (egui::pos2(right + small, lower_y), small),
            (top, big),
            (egui::pos2(right + Self::KNOBS_WIDE * tile - small, lower_y), small),
        ];
        let radius = Self::BUTTON * tile;
        let button_y = first.y + 2.0 * tile + gap + 0.26 * tile + radius;
        let buttons = [0.0, 1.0, 2.0].map(|column| {
            (egui::pos2(first.x + tile / 2.0 + column * (tile + gap), button_y), radius)
        });
        SmallPlan { first, tile, gap, knobs, buttons }
    }

    /// Where a control's own icon sits, which is what slides when two swap.
    pub(super) fn anchor(&self, cell: usize) -> egui::Pos2 {
        if let Some(knob) = small_knob_of_cell(cell) {
            let (center, radius) = self.knobs[knob];
            let middle = radius * (1.0 + Self::INNER) / 2.0;
            return match cell / 6 {
                0 => center - vec2(middle, 0.0),
                1 => center + vec2(middle, 0.0),
                _ => center,
            };
        }
        self.zone(cell).center()
    }

    /// The control under a point, knob parts and round buttons included.
    pub(super) fn cell_at(&self, at: egui::Pos2) -> Option<usize> {
        for (knob, (center, radius)) in self.knobs.iter().copied().enumerate() {
            let from_center = at.distance(center);
            if from_center <= radius {
                let [left, right, press] = small_knob_cells(knob);
                return Some(match (from_center <= radius * Self::INNER, at.x < center.x) {
                    (true, _) => press,
                    (false, true) => left,
                    (false, false) => right,
                });
            }
        }
        if let Some(button) = self.buttons.iter().position(|(center, radius)| at.distance(*center) <= *radius) {
            return Some(12 + button);
        }
        (0..6).map(small_cell_of_key).find(|cell| self.zone(*cell).expand(self.gap / 2.0).contains(at))
    }

    /// The part of the pad that belongs to a cell.
    pub(super) fn zone(&self, cell: usize) -> Rect {
        if small_cell_has_screen(cell) {
            let (row, column) = ((cell / 6) as f32, (cell % 6) as f32);
            return Rect::from_min_size(self.first + vec2(column, row) * (self.tile + self.gap), Vec2::splat(self.tile));
        }
        if let Some(knob) = small_knob_of_cell(cell) {
            let (center, radius) = self.knobs[knob];
            let circle = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
            return match cell / 6 {
                0 => circle.with_max_x(center.x),
                1 => circle.with_min_x(center.x),
                _ => Rect::from_center_size(center, Vec2::splat(radius * 2.0 * Self::INNER)),
            };
        }
        let (center, radius) = self.buttons[cell - 12];
        Rect::from_center_size(center, Vec2::splat(radius * 2.0))
    }
}

/// The cells of a block of keys, in reading order.
pub(super) fn cfg_block_cells(cols: usize, first: usize, span: [u8; 2]) -> Vec<usize> {
    let (across, down) = (span[0] as usize, span[1] as usize);
    (0..down).flat_map(|row| (0..across).map(move |column| first + row * cols + column)).collect()
}

/// The colour for small text on the front of a pad.
pub(super) fn on_face(face: [u8; 3]) -> Color32 {
    let light = face.iter().map(|c| *c as u32).sum::<u32>() / 3;
    if light > 128 { Color32::from_gray(50) } else { Color32::from_gray(190) }
}

/// A control's own icon, drawn with the window's fonts.
pub(super) fn paint_icon(painter: &egui::Painter, at: egui::Pos2, key: &DockKey, size: f32) {
    let color = Color32::from_gray(235);
    if let Some(c) = icons::character(&key.glyph) {
        let font = FontId::new(size, egui::FontFamily::Name(ICON_FONT.into()));
        painter.text(at, Align2::CENTER_CENTER, c, font, color);
    } else if !key.glyph.is_empty() {
        painter.text(at, Align2::CENTER_CENTER, &key.glyph, FontId::proportional(size * 0.85), color);
    }
}

/// What's left of a knob part or a round button while its action is moving.
pub(super) fn paint_chip(painter: &egui::Painter, rect: Rect, key: &DockKey) {
    let (center, radius) = (rect.center(), rect.width() / 2.0);
    painter.circle_filled(center, radius, Color32::from_gray(46));
    painter.circle_stroke(center, radius, Stroke::new(2.0, ACCENT));
    if key.glyph.is_empty() {
        let first = key.title.lines().next().unwrap_or_default();
        painter.text(center, Align2::CENTER_CENTER, first, FontId::proportional(11.0), Color32::from_gray(235));
    } else {
        paint_icon(painter, center, key, radius * 0.95);
    }
}

/// How a part of a control looks right now.
pub(super) fn part_color(state: (bool, bool, bool, bool)) -> Color32 {
    let (selected, hovered, dropping, pressed) = state;
    if pressed || dropping {
        Color32::from_gray(225)
    } else if selected {
        ACCENT
    } else if hovered {
        Color32::from_gray(76)
    } else {
        Color32::from_gray(46)
    }
}

/// An arc on one side of a knob, for turning that way.
pub(super) fn paint_turn(painter: &egui::Painter, center: egui::Pos2, radius: f32, thickness: f32, left: bool, color: Color32) {
    use std::f32::consts::PI;
    let start = if left { PI / 2.0 } else { -PI / 2.0 } + 0.14;
    let sweep = PI - 0.28;
    let points: Vec<egui::Pos2> = (0..=36)
        .map(|step| {
            let angle = start + sweep * step as f32 / 36.0;
            center + vec2(angle.cos(), angle.sin()) * radius
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(thickness, color)));
}

/// The cell nearest a point, measured from the top left of the first cell.
/// The gaps belong to the nearest cell, so nothing flickers between cells.
pub(super) fn cell_at(point: Vec2, tile: f32, gap: f32, rows: usize, cols: usize) -> usize {
    let line = |at: f32, count: usize| (((at + gap / 2.0) / (tile + gap)).floor().max(0.0) as usize).min(count - 1);
    line(point.y, rows) * cols + line(point.x, cols)
}

/// Where a pad's parts sit in a given space, for the editor and for pads on
/// screen alike.
pub(super) struct PadLayout {
    pub(super) device: Rect,
    pub(super) tile: f32,
    pub(super) gap: f32,
    pub(super) pad: f32,
    pub(super) first: egui::Pos2,
    rows: usize,
    cols: usize,
    /// The AKP03E's buttons and knobs; none on the 293S.
    pub(super) plan: Option<SmallPlan>,
}

impl PadLayout {
    /// The window's pad: as big as fits, drawn like a pad on screen.
    pub(super) fn new(kind: PadKind, rows: usize, cols: usize, area: Rect) -> PadLayout {
        Self::fitted(kind, rows, cols, area, (44.0, 118.0))
    }

    /// A pad with keys of this size, centred on a point. Its gaps and edges
    /// grow with the keys, as Home Assistant's card draws it, so it looks the
    /// same at any size and anywhere.
    pub(super) fn scaled(kind: PadKind, rows: usize, cols: usize, tile: f32, center: egui::Pos2) -> PadLayout {
        let gap = tile * kind.shown_gaps()[0] as f32 / 100.0;
        Self::build(kind, rows, cols, tile, (gap, tile * EDGE), center)
    }

    /// How round the front's corners are.
    pub(super) fn corner(&self) -> u8 {
        (self.tile * BODY_CORNER).min(127.0) as u8
    }

    /// The 293S's side screen, behind its band tiles: one strip, as on the pad.
    pub(super) fn band_strip(&self) -> Rect {
        self.band_tile(0).union(self.band_tile(2))
    }

    /// The biggest pad on screen that fits an area, its keys kept between
    /// these sizes.
    pub(super) fn fitted(kind: PadKind, rows: usize, cols: usize, area: Rect, (smallest, largest): (f32, f32)) -> PadLayout {
        let fits = |tile: f32| {
            let size = Self::scaled(kind, rows, cols, tile, egui::Pos2::ZERO).device.size();
            size.x <= area.width() + 0.5 && size.y <= area.height() + 0.5
        };
        let (mut low, mut high) = (smallest, largest);
        if fits(low) {
            for _ in 0..24 {
                let middle = (low + high) / 2.0;
                if fits(middle) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
        }
        Self::scaled(kind, rows, cols, low, area.center())
    }

    fn build(kind: PadKind, rows: usize, cols: usize, tile: f32, (gap, pad): (f32, f32), center: egui::Pos2) -> PadLayout {
        let has_band = kind.has_band();
        let band_share = if has_band { BAND_TILE } else { 0.0 };
        let gaps = if has_band { cols } else { cols - 1 } as f32;
        let size = if kind.is_grid() {
            vec2(
                cols as f32 * tile + gap * gaps + tile * band_share + pad * 2.0,
                rows as f32 * tile + gap * (rows - 1) as f32 + pad * 2.0,
            )
        } else {
            SmallPlan::device_size(tile, gap, pad)
        };
        let device = Rect::from_center_size(center, size);
        let plan = (!kind.is_grid()).then(|| SmallPlan::new(device, tile, gap, pad));
        PadLayout { device, tile, gap, pad, first: device.min + vec2(pad, pad), rows, cols, plan }
    }

    /// A cell's own place: a key, or on the AKP03E a button or part of a knob.
    pub(super) fn home(&self, cell: usize) -> Rect {
        match &self.plan {
            Some(plan) => plan.zone(cell),
            None => {
                let (row, column) = ((cell / self.cols) as f32, (cell % self.cols) as f32);
                Rect::from_min_size(self.first + vec2(column, row) * (self.tile + self.gap), Vec2::splat(self.tile))
            }
        }
    }

    /// One of the 293S's band tiles, to the right of the keys.
    pub(super) fn band_tile(&self, tile: usize) -> Rect {
        let width = self.tile * BAND_TILE;
        let left = self.device.max.x - self.pad - width;
        // Centred on the keys, as in the layout.
        let keys = self.rows as f32 * self.tile + (self.rows as f32 - 1.0).max(0.0) * self.gap;
        let band = band_height(self.gap / self.tile) * self.tile;
        let top = self.device.min.y + self.pad + ((keys - band) / 2.0).max(0.0) + tile as f32 * (self.tile + self.gap);
        Rect::from_min_size(egui::pos2(left, top), Vec2::splat(width))
    }

    /// The control under a point, if any.
    pub(super) fn cell_at(&self, at: egui::Pos2, rows: usize) -> Option<usize> {
        match &self.plan {
            Some(plan) => plan.cell_at(at),
            None => {
                let keys = self.home(0).union(self.home(self.rows * self.cols - 1));
                keys.contains(at).then(|| cell_at(at - self.first, self.tile, self.gap, rows, self.cols))
            }
        }
    }
}

/// The AKP03E's round buttons and knobs. `state` colours each part, and
/// `at_rest` leaves out the icons that are sliding elsewhere.
pub(super) fn paint_small_controls(
    painter: &egui::Painter,
    plan: &SmallPlan,
    keys: &[DockKey],
    mut state: impl FnMut(usize) -> Color32,
    caption: Color32,
    at_rest: impl Fn(usize) -> bool,
) {
    for (index, (center, radius)) in plan.buttons.iter().copied().enumerate() {
        let cell = 12 + index;
        painter.circle_filled(center, radius, state(cell));
        painter.circle_stroke(center, radius, Stroke::new(1.5, Color32::from_gray(96)));
        if at_rest(cell) {
            paint_icon(painter, center, &keys[cell], radius * 0.95);
        }
        let below = center + vec2(0.0, radius + 3.0);
        painter.text(below, Align2::CENTER_TOP, &keys[cell].title, FontId::proportional(11.5), caption);
    }
    for (knob, (center, radius)) in plan.knobs.iter().copied().enumerate() {
        let [left, right, press] = small_knob_cells(knob);
        let inner = radius * SmallPlan::INNER;
        let thickness = radius - inner - 5.0;
        let middle = (radius + inner) / 2.0;
        painter.circle_filled(center, radius, Color32::from_gray(26));
        painter.circle_stroke(center, radius, Stroke::new(1.5, Color32::from_gray(96)));
        paint_turn(painter, center, middle, thickness, true, state(left));
        paint_turn(painter, center, middle, thickness, false, state(right));
        painter.circle_filled(center, inner - 2.0, state(press));
        let size = (radius * 0.34).max(13.0);
        if at_rest(left) {
            paint_icon(painter, center - vec2(middle, 0.0), &keys[left], size);
        }
        if at_rest(right) {
            paint_icon(painter, center + vec2(middle, 0.0), &keys[right], size);
        }
        if at_rest(press) {
            paint_icon(painter, center, &keys[press], size * 1.1);
        }
        let below = center + vec2(0.0, radius + 2.0);
        painter.text(below, Align2::CENTER_TOP, SMALL_KNOBS[knob], FontId::proportional(11.5), caption);
    }
}

/// After two cells swap keys, each key's slide carries on from where it is
/// on screen. `shift` is the distance from cell `to` to cell `from`.
pub(super) fn swap_glides(glide: &mut HashMap<usize, Vec2>, from: usize, to: usize, shift: Vec2) {
    let carried = glide.remove(&from).unwrap_or(Vec2::ZERO);
    let displaced = glide.remove(&to).unwrap_or(Vec2::ZERO);
    glide.insert(to, carried + shift);
    glide.insert(from, displaced - shift);
}

/// The corners of a stretched camera's outline, which follow its picture.
pub(super) fn block_radius(block: Rect, key: Rect, style: &KeyStyle) -> CornerRadius {
    if style.shape == KeyShape::Circle {
        CornerRadius::same((block.width().min(block.height()) / 2.0).min(127.0) as u8)
    } else {
        key_radius(key, style)
    }
}

/// The corners of a band tile, and of the strip behind the tiles.
pub(super) fn band_radius(rect: Rect, style: &KeyStyle) -> CornerRadius {
    let shape = render::band_shape(style);
    let corner = match shape.shape {
        KeyShape::Square => 0.0,
        KeyShape::Rounded => shape.corner as f32 * rect.width() / render::BAND_SIZE as f32,
        // Round tiles, and round ends on the strip.
        KeyShape::Circle => rect.width() / 2.0,
    };
    CornerRadius::same(corner.min(127.0) as u8)
}

pub(super) fn key_radius(rect: Rect, style: &KeyStyle) -> CornerRadius {
    let corner = match style.shape {
        KeyShape::Square => 0.0,
        KeyShape::Rounded => style.corner as f32 * rect.width() / render::KEY_SIZE as f32,
        KeyShape::Circle => rect.width() / 2.0,
    };
    CornerRadius::same(corner.min(127.0) as u8)
}

impl DockApp {
    pub(super) fn page_tabs(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.horizontal(|ui| {
            let profile = cfg.profile_mut();
            let several = profile.pages.len() > 1;
            for i in 0..profile.pages.len() {
                let label = RichText::new(format!("Page {}", i + 1));
                let mut tab = ui.selectable_label(profile.active_page == i, label);
                if several {
                    tab = tab.on_hover_text("Right-click to delete it");
                }
                if tab.clicked() {
                    profile.active_page = i;
                    self.selected = None;
                    self.recording = None;
                }
                // Deleting is kept off the bar, where a slip would reach it.
                if several {
                    tab.context_menu(|ui| {
                        if ui.button(RichText::new(format!("Delete page {}…", i + 1)).color(c_warn())).clicked() {
                            profile.active_page = i;
                            self.selected = None;
                            self.recording = None;
                            self.confirm = Some(Confirm::DeletePage);
                            ui.close();
                        }
                    });
                }
            }
            if ui.button("+").on_hover_text("Add a page").clicked() {
                profile.add_page();
                self.selected = None;
            }
        });
    }

    /// Shows a picture drawn by the same code that draws for the pad.
    pub(super) fn drawn_texture(&mut self, ctx: &egui::Context, id: u64, draw: impl FnOnce() -> image::RgbImage) -> TextureHandle {
        let frame = self.frames;
        if self.drawn.len() > 400 {
            // Clocks and graphs make a new picture each minute or so: those
            // not shown lately go, and what's on screen stays.
            self.drawn.retain(|_, (_, used)| frame.saturating_sub(*used) < 10);
        }
        let (texture, used) = self.drawn.entry(id).or_insert_with(|| {
            let picture = draw();
            let size = [picture.width() as usize, picture.height() as usize];
            let pixels = egui::ColorImage::from_rgb(size, picture.as_raw());
            (ctx.load_texture(format!("drawn-{id}"), pixels, TextureOptions::LINEAR), frame)
        });
        *used = frame;
        texture.clone()
    }

    /// `offline_label` is the setting's value: the caller already holds the
    /// settings, and locking them again here would hang the window.
    pub(super) fn paint_key(&mut self, painter: &egui::Painter, rect: Rect, key: &DockKey, style: &KeyStyle, entities: &Entities, offline_label: bool, state: (bool, bool, bool)) {
        // The window shows Home Assistant's keys as the pads do.
        let ha = self.shared.ha.availability(offline_label);
        self.paint_look(painter, rect, Look::of_with(key, style, entities, ha), key, style, state);
    }

    pub(super) fn paint_look(&mut self, painter: &egui::Painter, rect: Rect, look: Option<Look>, key: &DockKey, style: &KeyStyle, state: (bool, bool, bool)) {
        let (selected, hovered, drop_target) = state;
        let texture = match &look {
            Some(look) => self.drawn_texture(painter.ctx(), hash_of(look), || render::key_picture(look)),
            None => self.drawn_texture(painter.ctx(), hash_of(style), || render::empty_picture(style)),
        };
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        // A stretched camera's picture has its shape already, so its keys stay square.
        let cut_already = look.as_ref().is_some_and(|look| {
            look.is_stretched() && look.camera.as_ref().is_some_and(|camera| camera.version > 0)
        });
        let radius = if cut_already { CornerRadius::ZERO } else { key_radius(rect, style) };
        // Painted with its corners cut, so it can slide over other keys.
        painter.add(egui::epaint::RectShape::filled(rect, radius, Color32::WHITE).with_texture(texture.id(), uv));
        if hovered && !drop_target {
            painter.rect_filled(rect, radius, Color32::from_white_alpha(18));
        }
        if look.is_none() && hovered {
            painter.text(rect.center(), Align2::CENTER_CENTER, "+", FontId::proportional(24.0), c_dim());
        }
        if !key.hotkey.is_empty() {
            let dot = rect.center_top() + vec2(0.0, 7.0);
            painter.circle_filled(dot, 3.0, Color32::from_white_alpha(200));
        }
        if drop_target {
            painter.rect_stroke(rect, radius, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        } else if selected {
            painter.rect_stroke(rect, radius, Stroke::new(2.5, ACCENT), StrokeKind::Outside);
        }
    }

    pub(super) fn grid(&mut self, ui: &mut Ui, cfg: &mut Config) {
        let pi = cfg.active_profile;
        let pg = cfg.profile().active_page;
        let (rows, cols) = (cfg.profile().rows, cfg.profile().cols);
        let area = ui.available_rect_before_wrap();
        let shared = self.shared.clone();
        let entities = lock(&shared.ha.entities);
        let style = cfg.profile().style_on_pad(&cfg.style);
        let kind = cfg.profile().pad;
        let pad_id = cfg.shown_pad_id();
        let has_band = kind.has_band();
        let on_its_pad = cfg.pad_profile_of(&pad_id) == Some(pi);
        // On an AKP03E only its screen keys can swap places: the plain buttons
        // and knobs are labelled for where they are.
        let covers: Vec<Option<Cover>> = (0..rows * cols).map(|cell| cfg.profile().cover(pg, cell)).collect();
        let on_screen = |cell: usize| kind.cell_has_screen(cell);
        let can_move = |cell: usize| covers[cell].is_none_or(|cover| cover.owner == cell);
        let layout = PadLayout::new(kind, rows, cols, area);
        let (tile, gap, device_rect, first, plan) = (layout.tile, layout.gap, layout.device, layout.first, layout.plan);
        // A see-through front shows the window behind it, with only the
        // outline to show where the pad is.
        let background = ui.visuals().panel_fill;
        let front = cfg.shown_pad().and_then(|entry| entry.front());
        let face = front.unwrap_or([background.r(), background.g(), background.b()]);
        if front.is_some() {
            ui.painter().rect_filled(device_rect, layout.corner(), Color32::from_rgb(face[0], face[1], face[2]));
        }
        ui.painter().rect_stroke(device_rect, layout.corner(), Stroke::new(1.0, c_line()), StrokeKind::Inside);

        let count = rows * cols;
        let home = |i: usize| layout.home(i);
        // Where a key's content sits, which is what slides when two swap.
        let spot = |i: usize| match &plan {
            Some(plan) if !on_screen(i) => plan.anchor(i),
            _ => home(i).center(),
        };
        let chip = Vec2::splat((tile * 0.62).max(34.0));
        let slider = |i: usize| if on_screen(i) { home(i) } else { Rect::from_center_size(spot(i), chip) };
        let ctx = ui.ctx().clone();
        if self.glide_place != (pi, pg) {
            self.glide_place = (pi, pg);
            self.glide.clear();
            self.carried = None;
        }

        // The key being dragged, and the cell it would land in. Leaving the
        // keys altogether means it would go back where it came from.
        let mut moving = egui::DragAndDrop::payload::<MoveKey>(&ctx).map(|moved| *moved);
        if moving.is_some_and(|moved| moved.place != (pi, pg) || moved.cell >= count) {
            egui::DragAndDrop::clear_payload(&ctx);
            moving = None;
        }
        let pointer = ctx.pointer_latest_pos();
        // A stretched camera answers for all the keys it covers.
        let whole = |cell: usize, span: [u8; 2]| {
            home(cell).union(home(cell + (span[1] as usize - 1) * cols + span[0] as usize - 1))
        };
        let keys_area = home(0).union(home(count - 1)).expand(gap / 2.0);
        // A stretched camera moves as a block, placed by its first key.
        let block = moving.and_then(|moved| covers[moved.cell].filter(|cover| cover.owner == moved.cell));
        let target = moving.zip(pointer).and_then(|(moved, at)| {
            let at = if block.is_some() { at - moved.grab } else { at };
            let cell = match &plan {
                Some(plan) => plan.cell_at(at),
                None => (block.is_some() || keys_area.contains(at)).then(|| cell_at(at - first, tile, gap, rows, cols)),
            }?;
            match block {
                Some(cover) => Some(cfg.profile().block_home(cell, cover.span)),
                None => covers[cell].is_none().then_some(cell),
            }
        });
        // Every key that would change place if this were dropped now.
        let source: Vec<usize> = moving
            .zip(block)
            .map_or(Vec::new(), |(moved, cover)| cfg.profile().block_cells(moved.cell, cover.span));
        let moves: Vec<(usize, usize)> = match (moving, block, target) {
            (Some(moved), Some(cover), Some(to)) => {
                cfg.profile().block_moves(pg, moved.cell, cover.span, to).unwrap_or_default()
            }
            (Some(moved), None, Some(to)) if moved.cell != to => vec![(moved.cell, to), (to, moved.cell)],
            _ => Vec::new(),
        };

        // Behind the keys: dragging from here draws a box to choose keys with.
        let backdrop = ui.interact(device_rect, Id::NULL.with(("backdrop", pi, pg)), Sense::click_and_drag());
        if backdrop.drag_started() && moving.is_none() {
            self.boxing = ctx.input(|input| input.pointer.press_origin());
        }
        if backdrop.clicked() {
            self.selected = None;
            self.selected_tile = None;
        }
        let chosen = self.selection(cfg);
        let has_clip = self.clip.is_some();
        let mut clicked = None;
        let mut command = None;
        let mut hovers = vec![(false, false); count];
        for i in 0..count {
            if covers[i].is_some_and(|cover| cover.owner != i) {
                continue;
            }
            let rect = covers[i].map_or(home(i), |cover| whole(i, cover.span));
            let response = ui.interact(rect, Id::NULL.with(("key", pi, pg, i)), Sense::click_and_drag());
            let keys = &mut cfg.profiles[pi].pages[pg].keys;

            if !keys[i].is_empty() && can_move(i) {
                let picked_at = ctx.input(|input| input.pointer.press_origin()).unwrap_or(rect.center());
                // Measured from the first key, so a block stays under the pointer.
                let grab = if on_screen(i) { picked_at - home(i).center() } else { Vec2::ZERO };
                response.dnd_set_drag_payload(MoveKey { place: (pi, pg), cell: i, grab });
            } else if keys[i].is_empty() && response.drag_started() && moving.is_none() {
                // An empty key can't move, so dragging from it draws a box.
                self.boxing = ctx.input(|input| input.pointer.press_origin());
            }
            hovers[i] = (response.hovered() && moving.is_none(), response.dnd_hover_payload::<NewAction>().is_some());
            let response = if on_screen(i) {
                response
            } else {
                response.on_hover_text(keys[i].title.replace('\n', ", "))
            };
            if let Some(dropped) = response.dnd_release_payload::<NewAction>() {
                add_library_action(&mut keys[i], dropped.0);
                self.selected = Some(i);
                self.selected_tile = None;
            }
            if response.clicked() {
                clicked = Some((i, ctx.input(|input| input.modifiers)));
            }
            if response.double_clicked() {
                self.run(&keys[i], pi);
            }
            // Right-clicking one of several chosen keys acts on all of them.
            let group = chosen.len() > 1 && chosen.contains(&i);
            let filled = group || !keys[i].is_empty();
            let many = if group { format!(" {} keys", chosen.len()) } else { String::new() };
            response.context_menu(|ui| {
                if !group {
                    if ui.button("Run").clicked() {
                        self.run(&keys[i], pi);
                        ui.close();
                    }
                    ui.separator();
                }
                let items = [
                    ("Cut", "Ctrl+X", KeyCommand::Cut, filled),
                    ("Copy", "Ctrl+C", KeyCommand::Copy, filled),
                    ("Paste", "Ctrl+V", KeyCommand::Paste, has_clip),
                    ("Duplicate", "Ctrl+D", KeyCommand::Duplicate, filled),
                ];
                for (label, keys_for_it, wanted, enabled) in items {
                    let label = if wanted == KeyCommand::Paste { label.to_string() } else { format!("{label}{many}") };
                    if ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(keys_for_it)).clicked() {
                        command = Some((i, wanted));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.add_enabled(filled, egui::Button::new(format!("Delete{many}")).shortcut_text("Del")).clicked() {
                    command = Some((i, KeyCommand::Delete));
                    ui.close();
                }
            });
        }
        if let Some((cell, modifiers)) = clicked {
            self.click_key(cfg, cell, modifiers);
        }
        if let Some((cell, wanted)) = command {
            self.key_command(cfg, cell, wanted);
        }

        // The corner of a selected camera, to drag it over more keys.
        let stretching = self.selected.filter(|_| chosen.len() <= 1).filter(|cell| {
            *cell < count && cfg.profile().pages[pg].keys[*cell].camera().is_some() && on_screen(*cell)
                && !covers[*cell].is_some_and(|cover| cover.owner != *cell)
        });
        let mut grip = None;
        if let Some(cell) = stretching {
            let most = cfg.profile().largest_span(cell);
            let span = covers[cell].map_or([1, 1], |cover| cover.span);
            let corner = Rect::from_center_size(whole(cell, span).max, Vec2::splat(22.0));
            let response = ui.interact(corner, Id::NULL.with(("stretch", pi, pg, cell)), Sense::drag());
            if response.hovered() || response.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            if let (true, Some(at)) = (response.dragged(), pointer) {
                let from = at - home(cell).min + Vec2::splat(gap);
                let wanted = |room: f32, most: u8| ((room / (tile + gap)).ceil().max(1.0) as u8).min(most);
                cfg.profiles[pi].pages[pg].keys[cell].span = [wanted(from.x, most[0]), wanted(from.y, most[1])];
            }
            grip = Some(corner.center());
        }

        let chosen = self.selection(cfg);
        // The page being edited, which need not be the one the pad is on.
        let showing = frames::Showing { profile: pi, page: pg, outside: face };
        let looks = frames::pad_frame_showing(&shared, cfg, &entities, &pad_id, showing).unwrap_or_default().looks(count);
        let keys = &mut cfg.profiles[pi].pages[pg].keys;
        if moving.is_some() && ctx.input(|input| input.pointer.any_released()) {
            if let (Some(MoveKey { cell: from, .. }), Some(to)) = (moving, target) {
                if !moves.is_empty() {
                    apply_moves(keys, &moves);
                    if block.is_some() {
                        for (key_from, key_to) in moves.iter().copied().filter(|(cell, _)| !source.contains(cell)) {
                            let now = self.glide.remove(&key_from).unwrap_or(Vec2::ZERO);
                            self.glide.insert(key_to, now + (spot(key_from) - spot(key_to)));
                        }
                        let now = self.glide.remove(&from).unwrap_or(Vec2::ZERO);
                        self.glide.insert(to, now + (spot(from) - spot(to)));
                    } else {
                        swap_glides(&mut self.glide, from, to, spot(from) - spot(to));
                    }
                    self.selected = Some(to);
                    self.selected_tile = None;
                    self.recording = None;
                    self.carried = Some(to);
                }
            }
            moving = None;
        }

        let seconds = ctx.input(|input| input.stable_dt).min(0.05);
        for i in 0..count {
            let wanted = match moving {
                Some(MoveKey { cell, grab, .. }) if cell == i => {
                    if let Some(at) = pointer {
                        self.glide.insert(i, at - grab - spot(i));
                    }
                    continue;
                }
                Some(_) => match moves.iter().find(|(from, _)| *from == i) {
                    Some((_, to)) if !source.contains(&i) && !keys[i].is_empty() => spot(*to) - spot(i),
                    _ => Vec2::ZERO,
                },
                _ => Vec2::ZERO,
            };
            let now = self.glide.get(&i).copied().unwrap_or(Vec2::ZERO);
            let next = glide_toward(now, wanted, seconds);
            if next == Vec2::ZERO {
                self.glide.remove(&i);
            } else {
                self.glide.insert(i, next);
            }
        }
        if let Some(MoveKey { cell, .. }) = moving {
            self.carried = Some(cell);
        } else if self.carried.is_some_and(|cell| !self.glide.contains_key(&cell)) {
            self.carried = None;
        }
        if !self.glide.is_empty() {
            ctx.request_repaint();
        }

        // Cells and resting keys first, then sliding keys, then the carried key.
        let painter = ui.painter().clone();
        let empty = DockKey::default();
        let lifted: Vec<usize> = self.carried.map_or(Vec::new(), |cell| {
            covers.get(cell).copied().flatten().filter(|cover| cover.owner == cell).map_or(Vec::new(), |cover| {
                cfg_block_cells(cols, cell, cover.span)
            })
        });
        for i in (0..count).filter(|i| on_screen(*i)) {
            let (hovered, new_action) = hovers[i];
            let landing = moves.iter().any(|(from, to)| *to == i && source.contains(from));
            let drop_here = new_action || (moving.is_some() && (target == Some(i) || landing));
            if self.glide.contains_key(&i) || lifted.contains(&i) {
                self.paint_key(&painter, home(i), &empty, &style, &entities, cfg.ha_offline_label, (false, false, drop_here));
                continue;
            }
            let selected = chosen.contains(&i) && covers[i].is_none();
            let owner = covers[i].map_or(i, |cover| cover.owner);
            self.paint_look(&painter, home(i), looks[i].clone(), &keys[owner], &style, (selected, hovered, drop_here));
            let pressed_on_pad = on_its_pad && actions::just_pressed(&self.shared, kind, &pad_id, i);
            if pressed_on_pad {
                painter.rect_filled(home(i), 12, Color32::from_white_alpha(70));
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
        }
        let at_rest = self.carried.is_none();
        for cell in chosen.iter().copied().filter(|_| at_rest) {
            if let Some(cover) = covers.get(cell).copied().flatten() {
                let edge = whole(cover.owner, cover.span);
                painter.rect_stroke(edge, block_radius(edge, home(cell), &style), Stroke::new(2.5, ACCENT), StrokeKind::Outside);
            }
        }
        if let Some(at) = grip.filter(|_| at_rest) {
            painter.circle_filled(at, 8.0, ACCENT);
            painter.circle_stroke(at, 8.0, Stroke::new(2.0, Color32::WHITE));
        }
        if let Some(plan) = &plan {
            let state = |cell: usize| {
                let (hovered, dropping) = hovers[cell];
                let dropping = dropping || (moving.is_some() && target == Some(cell));
                let pressed = on_its_pad && small_pad::just_pressed(cell);
                if pressed {
                    ctx.request_repaint_after(std::time::Duration::from_millis(80));
                }
                part_color((chosen.contains(&cell), hovered, dropping, pressed))
            };
            paint_small_controls(&painter, plan, keys, state, on_face(face), |cell| !self.glide.contains_key(&cell));
        }
        for i in 0..count {
            if let Some(shift) = self.glide.get(&i).copied().filter(|_| self.carried != Some(i) && !keys[i].is_empty()) {
                if on_screen(i) {
                    let state = (chosen.contains(&i), false, false);
                    self.paint_key(&painter, home(i).translate(shift), &keys[i], &style, &entities, cfg.ha_offline_label, state);
                } else {
                    paint_chip(&painter, slider(i).translate(shift), &keys[i]);
                }
            }
        }
        let lift = ctx.animate_bool_with_time_and_easing(
            Id::NULL.with("key-lift"),
            moving.is_some(),
            0.14,
            egui::emath::easing::cubic_out,
        );
        if let Some(cell) = self.carried {
            let shift = self.glide.get(&cell).copied().unwrap_or(Vec2::ZERO);
            let moved = slider(cell).translate(shift);
            let rect = Rect::from_center_size(moved.center(), moved.size() * (1.0 + 0.08 * lift));
            let top = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::NULL.with("carried-key")));
            if lifted.is_empty() {
                let radius = if on_screen(cell) {
                    key_radius(rect, &style)
                } else {
                    CornerRadius::same((rect.width() / 2.0).min(127.0) as u8)
                };
                let shadow = Color32::from_black_alpha((150.0 * lift) as u8);
                top.add(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: shadow }.as_shape(rect, radius));
                if on_screen(cell) {
                    let state = (chosen.contains(&cell), false, false);
                    self.paint_key(&top, rect, &keys[cell], &style, &entities, cfg.ha_offline_label, state);
                } else {
                    paint_chip(&top, rect, &keys[cell]);
                }
                // A black key would otherwise vanish against the black pad.
                let edge = Color32::from_white_alpha((70.0 * lift) as u8);
                top.rect_stroke(rect, radius, Stroke::new(1.0, edge), StrokeKind::Outside);
            } else {
                // The whole picture travels, slice by slice.
                for slice in lifted.iter().copied() {
                    let state = (false, false, false);
                    self.paint_look(&top, home(slice).translate(shift), looks[slice].clone(), &keys[cell], &style, state);
                }
                let edge = lifted.iter().fold(home(cell), |edge, slice| edge.union(home(*slice))).translate(shift);
                top.rect_stroke(edge, block_radius(edge, home(cell), &style), Stroke::new(2.0, ACCENT), StrokeKind::Outside);
            }
        }
        if has_band {
            self.band_tiles(ui, cfg, &entities, &layout);
        }
        drop(entities);

        // A box being drawn to choose keys, and the keys inside it once let go.
        if let Some(start) = self.boxing {
            let rect = Rect::from_two_pos(start, pointer.unwrap_or(start));
            if ctx.input(|input| input.pointer.primary_down()) {
                let top = ctx.layer_painter(LayerId::new(Order::Foreground, Id::NULL.with("choosing-box")));
                top.rect_filled(rect, 4, ACCENT.gamma_multiply(0.18));
                top.rect_stroke(rect, 4, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
                ctx.request_repaint();
            } else {
                let inside: Vec<usize> = (0..count).filter(|cell| home(*cell).intersects(rect)).collect();
                let add = ctx.input(|input| input.modifiers.command || input.modifiers.shift);
                self.box_select(cfg, inside, add);
                self.boxing = None;
            }
        }
    }

    /// The band's tiles, placed to the right of the keys like on the pad.
    pub(super) fn band_tiles(&mut self, ui: &mut Ui, cfg: &Config, entities: &Entities, layout: &PadLayout) {
        ui.painter().rect_filled(layout.band_strip(), band_radius(layout.band_tile(0), &cfg.style), Color32::BLACK);
        for (i, face) in band::faces_now(&self.shared, cfg, entities).iter().enumerate() {
            let rect = layout.band_tile(i);
            let response = ui.interact(rect, Id::NULL.with(("band", i)), Sense::click());
            response.clone().on_hover_text("Side band: shows information, can't be pressed");
            self.paint_face(ui, rect, face, &cfg.style);
            if self.selected_tile == Some(i) {
                ui.painter().rect_stroke(rect, 6, Stroke::new(2.5, ACCENT), StrokeKind::Outside);
            } else if response.hovered() {
                ui.painter().rect_stroke(rect, 6, Stroke::new(1.0, c_dim()), StrokeKind::Inside);
            }
            if response.clicked() {
                self.selected_tile = Some(i);
                self.selected = None;
                self.recording = None;
            }
        }
        // Keeps the clock and the measurements moving on screen.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }

    pub(super) fn paint_face(&mut self, ui: &Ui, rect: Rect, face: &Face, style: &KeyStyle) {
        let id = hash_of(&(face, style));
        let texture = self.drawn_texture(ui.ctx(), id, || render::band_picture(face, style));
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
    }

    pub(super) fn drag_preview(&self, ctx: &egui::Context) {
        let Some(payload) = egui::DragAndDrop::payload::<NewAction>(ctx) else { return };
        let Some(pos) = ctx.pointer_interact_pos() else { return };
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::NULL.with("drag-preview")));
        let text = LIBRARY[payload.0].title;
        let rect = Rect::from_min_size(pos + vec2(12.0, 12.0), vec2(9.0 * text.len() as f32 + 16.0, 28.0));
        painter.rect_filled(rect, 7, ACCENT);
        painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.5), Color32::WHITE);
    }
}
