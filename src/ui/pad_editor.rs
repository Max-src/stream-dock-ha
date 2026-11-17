//! Adding or changing a virtual pad, in one window: its name, two sliders for
//! its keys, and the pad drawn as it will be while they move. Changing an
//! existing pad shows its keys where they are, and which would be removed.

use super::*;

/// What a virtual pad is, wherever one is offered.
pub(super) const VIRTUAL_HINT: &str = "A pad with no hardware: it exists only on your screen and in Home Assistant. Set up its keys like any pad's, open it with On screen, or show it on a Home Assistant dashboard card.";

/// The gap between keys and the front round them, in key widths, as the app
/// draws a virtual pad (model/layout.rs).
const GAP: f32 = 0.25;
const EDGE: f32 = 0.3;

/// A virtual pad being added (no id yet) or changed.
#[derive(Clone)]
pub(super) struct PadEditor {
    pub pad_id: Option<String>,
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    /// Just opened: type in the name straight away.
    pub focus: bool,
}

enum Chosen {
    Save,
    Cancel,
    Delete,
}

impl DockApp {
    /// Opens the window for a new virtual pad.
    pub(super) fn start_new_virtual_pad(&mut self, cfg: &Config) {
        let name = cfg.free_pad_name(PadKind::Dashboard.label());
        self.pad_editor = Some(PadEditor { pad_id: None, name, rows: 2, cols: 4, focus: true });
    }

    /// Opens the window for a virtual pad that's there.
    pub(super) fn edit_virtual_pad(&mut self, cfg: &Config, pad_id: &str) {
        let Some(entry) = cfg.pad(pad_id).filter(|entry| entry.kind == PadKind::Dashboard) else { return };
        let Some(running) = cfg.pad_profile_of(pad_id) else { return };
        let profile = &cfg.profiles[running];
        self.pad_editor = Some(PadEditor {
            pad_id: Some(pad_id.to_string()),
            name: entry.name.clone(),
            rows: profile.rows,
            cols: profile.cols,
            focus: false,
        });
    }

    pub(super) fn pad_editor_dialog(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        let Some(editor) = &mut self.pad_editor else { return };
        // The pad as it is, when changing one: its grid and the keys on the page it shows.
        let existing = editor.pad_id.as_deref().and_then(|id| cfg.pad_profile_of(id)).map(|i| &cfg.profiles[i]);
        let before = existing.map(|p| (p.rows, p.cols));
        let filled: Vec<bool> = existing.map_or(Vec::new(), |p| p.active_keys().iter().map(|key| !key.is_empty()).collect());
        let lost = editor.pad_id.as_deref().map_or(0, |id| cfg.keys_outside_grid(id, editor.rows, editor.cols));
        let name = editor.name.trim().to_string();
        let problem = if name.is_empty() {
            Some("Give it a name.")
        } else if cfg.pads.iter().any(|entry| entry.name == name && Some(&entry.id) != editor.pad_id.as_ref()) {
            Some("Another pad already has this name.")
        } else {
            None
        };
        let mut chosen = None;
        Modal::new(Id::NULL.with("pad editor")).show(ctx, |ui| {
            ui.set_width(440.0);
            let title = if editor.pad_id.is_some() { "Edit virtual pad" } else { "New virtual pad" };
            ui.label(RichText::new(title).size(17.0).strong());
            if editor.pad_id.is_none() {
                ui.label(dim(VIRTUAL_HINT).size(12.5));
            }
            ui.add_space(10.0);

            ui.label(dim("Name"));
            let field = text_field(ui, TextEdit::singleline(&mut editor.name).desired_width(f32::INFINITY).hint_text("Kitchen, Desk, Lights…"));
            if std::mem::take(&mut editor.focus) {
                field.request_focus();
            }
            ui.add_space(8.0);

            ui.style_mut().spacing.slider_width = 250.0;
            ui.add(egui::Slider::new(&mut editor.cols, 1..=10).text("keys across"));
            ui.add(egui::Slider::new(&mut editor.rows, 1..=8).text("keys down"));
            ui.add_space(8.0);

            draw_preview(ui, editor.rows, editor.cols, before, &filled);

            let count = editor.rows * editor.cols;
            ui.label(dim(format!("{count} key{} · numbered 1 to {count}, left to right then down, as Home Assistant counts them", if count == 1 { "" } else { "s" })).size(12.5));
            if lost > 0 {
                let what = if lost == 1 { "1 key that's set up is" } else { "keys that are set up are" };
                let what = if lost == 1 { what.to_string() } else { format!("{lost} {what}") };
                ui.label(RichText::new(format!("{what} outside the new size, counting every profile and page, and will be removed. Make it bigger to keep them.")).color(c_warn()).size(12.5));
            } else if before.is_some_and(|(_, cols)| cols != editor.cols) {
                ui.label(dim("Keys keep their row and column. Their numbers change, so check Home Assistant cards that name keys by number.").size(12.5));
            }
            if let Some(problem) = problem {
                ui.label(RichText::new(problem).color(c_warn()).size(12.5));
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                let save = if editor.pad_id.is_some() { "Save" } else { "Add the pad" };
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.add_enabled(problem.is_none(), egui::Button::new(RichText::new(save).strong()).fill(ACCENT)).clicked() || (enter && problem.is_none()) {
                    chosen = Some(Chosen::Save);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    chosen = Some(Chosen::Cancel);
                }
                if editor.pad_id.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Delete this pad…").color(c_warn())).clicked() {
                            chosen = Some(Chosen::Delete);
                        }
                    });
                }
            });
        });
        let Some(chosen) = chosen else { return };
        let Some(editor) = self.pad_editor.take() else { return };
        match chosen {
            Chosen::Cancel => {}
            Chosen::Delete => {
                if let Some(pad_id) = editor.pad_id {
                    self.confirm = Some(Confirm::RemovePad(pad_id));
                }
            }
            Chosen::Save => {
                match &editor.pad_id {
                    None => {
                        cfg.add_dashboard_pad(&name, editor.rows, editor.cols);
                        self.shared.set_status(format!("\"{name}\" is added and shown. Click a key to set it up."));
                    }
                    Some(pad_id) => {
                        if let Err(problem) = cfg.rename_pad(pad_id, &name) {
                            self.shared.set_status(problem);
                        }
                        cfg.set_dashboard_grid(pad_id, editor.rows, editor.cols);
                        cfg.show_pad_id(pad_id);
                    }
                }
                self.selected = None;
                self.selected_tile = None;
                self.recording = None;
                self.renaming = None;
            }
        }
    }
}

/// The pad as it will look: its front, and a numbered key in each place.
/// Changing a pad, keys with something on them are filled, and places the new
/// size leaves out are drawn outside the front, red where a key would go.
fn draw_preview(ui: &mut Ui, rows: usize, cols: usize, before: Option<(usize, usize)>, filled: &[bool]) {
    let (old_rows, old_cols) = before.unwrap_or((rows, cols));
    let (all_rows, all_cols) = (rows.max(old_rows), cols.max(old_cols));
    let units = |n: usize| n as f32 + (n.saturating_sub(1)) as f32 * GAP + 2.0 * EDGE;
    let room = vec2(ui.available_width(), 210.0);
    let key = (room.x / units(all_cols)).min(room.y / units(all_rows)).min(56.0);
    let (area, _) = ui.allocate_exact_size(vec2(room.x, units(all_rows) * key), Sense::hover());
    let painter = ui.painter_at(area.expand(2.0));
    let origin = area.center_top() - vec2(units(all_cols) * key / 2.0, 0.0);
    let front = Rect::from_min_size(origin, vec2(units(cols) * key, units(rows) * key));
    painter.rect_filled(front, key * EDGE, c_card());
    painter.rect_stroke(front, key * EDGE, egui::Stroke::new(1.0, c_line()), egui::StrokeKind::Inside);
    for r in 0..all_rows {
        for c in 0..all_cols {
            let at = origin + vec2(EDGE + c as f32 * (1.0 + GAP), EDGE + r as f32 * (1.0 + GAP)) * key;
            let cell = Rect::from_min_size(at, vec2(key, key));
            let inside = r < rows && c < cols;
            let was = r < old_rows && c < old_cols;
            let has = was && filled.get(r * old_cols + c).copied().unwrap_or(false);
            let corner = key * 0.18;
            if inside {
                let fill = if has { ACCENT.gamma_multiply(0.85) } else { c_line() };
                painter.rect_filled(cell, corner, fill);
                let number = (r * cols + c + 1).to_string();
                let size = (key * 0.34).clamp(9.0, 16.0);
                let colour = if has { Color32::WHITE } else { c_dim() };
                painter.text(cell.center(), egui::Align2::CENTER_CENTER, number, egui::FontId::proportional(size), colour);
            } else if has {
                let red = Color32::from_rgb(220, 80, 70);
                painter.rect_stroke(cell, corner, egui::Stroke::new(1.5, red), egui::StrokeKind::Inside);
                let pad = key * 0.3;
                let stroke = egui::Stroke::new(2.0, red);
                painter.line_segment([cell.min + vec2(pad, pad), cell.max - vec2(pad, pad)], stroke);
                painter.line_segment([cell.left_bottom() + vec2(pad, -pad), cell.right_top() + vec2(-pad, pad)], stroke);
            } else {
                painter.rect_stroke(cell, corner, egui::Stroke::new(1.0, c_line().gamma_multiply(0.6)), egui::StrokeKind::Inside);
            }
        }
    }
    ui.add_space(6.0);
}
