//! Reusable controls: colour pickers, state colours, picture modes, icons and text fields.

use super::*;

/// How a picture is shown.
pub(super) fn picture_fit_choice(ui: &mut Ui, id: &str, fit: &mut PictureFit) {
    ui.horizontal(|ui| {
        ui.label(dim("Show picture").size(12.5));
        ComboBox::from_id_salt(id).selected_text(fit.label()).width(190.0).show_ui(ui, |ui| {
            for choice in PictureFit::ALL {
                ui.selectable_value(fit, choice, choice.label()).on_hover_text(choice.hint());
            }
        });
    });
}

/// Text drawn from the icon set.
pub(super) fn icon_text(c: char, size: f32) -> RichText {
    RichText::new(c.to_string()).family(egui::FontFamily::Name(ICON_FONT.into())).size(size)
}

/// A button showing a key's icon, opening a search through all the icons.
pub(super) fn icon_choice(ui: &mut Ui, search: &mut String, key: &mut DockKey) {
    let shown = match icons::character(&key.glyph) {
        Some(c) => icon_text(c, 17.0),
        None if key.glyph.is_empty() => RichText::new("Icon…"),
        None => RichText::new(key.glyph.clone()).size(15.0),
    };
    let button = ui.selectable_label(!key.glyph.is_empty(), shown);
    let mut chosen = None;
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(340.0);
            let hint = format!("Search {} icons: fan, door, thermometer…", icons::count());
            let field = text_field(ui, TextEdit::singleline(search).desired_width(f32::INFINITY).hint_text(hint));
            if button.clicked() {
                field.request_focus();
            }
            ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    let found = icons::search(search, 150);
                    if found.is_empty() {
                        ui.label(dim("No icon has that in its name. The names are in English."));
                    }
                    for (name, c) in found {
                        let tile = egui::Button::new(icon_text(c, 22.0)).min_size(Vec2::splat(36.0));
                        if ui.add(tile).on_hover_text(name.trim_start_matches("mdi:")).clicked() {
                            chosen = Some(name);
                        }
                    }
                });
                if search.trim().is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for g in GLYPHS {
                            let tile = egui::Button::new(RichText::new(g).size(18.0)).min_size(Vec2::splat(36.0));
                            if ui.add(tile).clicked() {
                                chosen = Some(g.to_string());
                            }
                        }
                    });
                }
            });
            if chosen.is_some() {
                ui.close();
            }
        });
    if let Some(glyph) = chosen {
        key.glyph = glyph;
        key.image = None;
        key.no_icon = false;
    }
}

/// The preset colours, with the chosen one outlined.
pub(super) fn swatches(ui: &mut Ui, color: &mut [u8; 3]) {
    ui.spacing_mut().item_spacing.x = 5.0;
    for c in SWATCHES {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
        ui.painter().rect_filled(rect, 5, rgb(c));
        // The thin edge is what lets black show on the dark theme.
        let edge = if *color == c { Stroke::new(2.0, c_text()) } else { Stroke::new(1.0, c_dim()) };
        ui.painter().rect_stroke(rect, 5, edge, StrokeKind::Outside);
        if response.clicked() {
            *color = c;
        }
    }
}

/// A coloured button that opens the preset colours and a free colour picker.
pub(super) fn color_choice(ui: &mut Ui, color: &mut [u8; 3]) {
    let button = egui::Button::new("").fill(rgb(*color)).stroke(Stroke::new(1.0, c_dim())).min_size(vec2(40.0, 22.0));
    let button = ui.add(button);
    egui::Popup::menu(&button)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.horizontal(|ui| swatches(ui, color));
            let mut picked = rgb(*color);
            if egui::color_picker::color_picker_color32(ui, &mut picked, egui::color_picker::Alpha::Opaque) {
                *color = [picked.r(), picked.g(), picked.b()];
            }
        });
}

/// Two optional colours, one for each state of an entity.
pub(super) fn state_colors(ui: &mut Ui, hint: &str, choices: [(&str, &mut Option<[u8; 3]>, [u8; 3]); 2]) {
    ui.horizontal_wrapped(|ui| {
        for (label, color, usual) in choices {
            let mut chosen = color.is_some();
            if ui.checkbox(&mut chosen, label).on_hover_text(hint).changed() {
                *color = chosen.then_some(usual);
            }
            if let Some(color) = color {
                color_choice(ui, color);
            }
            ui.add_space(8.0);
        }
    });
}

/// One row for each exact state that has a colour of its own. On a key the
/// colour is the background, and the text can have one too.
pub(super) fn state_color_rows(ui: &mut Ui, rules: &mut Vec<StateColor>, with_text: bool) {
    let mut remove = None;
    for (row, rule) in rules.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            color_choice(ui, &mut rule.color);
            ui.label("when the state is");
            text_field(ui, TextEdit::singleline(&mut rule.state).desired_width(130.0).hint_text("heating, unavailable…"));
            if with_text {
                let mut own = rule.text.is_some();
                if ui.checkbox(&mut own, "Text").on_hover_text("A text colour for this state").changed() {
                    rule.text = own.then_some([255, 255, 255]);
                }
                if let Some(color) = &mut rule.text {
                    color_choice(ui, color);
                }
            }
            if ui.small_button("×").on_hover_text("Remove this colour").clicked() {
                remove = Some(row);
            }
        });
    }
    if let Some(row) = remove {
        rules.remove(row);
    }
}

/// A new state colour, starting from the state showing now if that's a word
/// nothing has claimed yet.
pub(super) fn add_state_color(rules: &mut Vec<StateColor>, state: &str) {
    let word = state.trim().parse::<f32>().is_err() && color_of_state(rules, state).is_none();
    let state = if word { state.trim().to_string() } else { String::new() };
    rules.push(StateColor { state, color: SWATCHES[4], text: None });
}

/// Background colours that follow a number or a state, such as a temperature.
pub(super) fn value_colors(ui: &mut Ui, a: &mut HaAction, state: &str) {
    if !a.level_colors.is_empty() || !a.state_colors.is_empty() {
        ui.label(dim("By value: an exact state wins, then the highest number reached.").size(12.5));
    }
    let mut remove = None;
    for (row, level) in a.level_colors.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            color_choice(ui, &mut level.color);
            ui.label("from");
            ui.add(egui::DragValue::new(&mut level.from).speed(0.5).max_decimals(2));
            if ui.small_button("×").on_hover_text("Remove this colour").clicked() {
                remove = Some(row);
            }
        });
    }
    if let Some(row) = remove {
        a.level_colors.remove(row);
    }
    state_color_rows(ui, &mut a.state_colors, true);
    ui.horizontal(|ui| {
        if ui.button("Colour from a number").on_hover_text("For values such as a temperature").clicked() {
            let top = a.level_colors.iter().map(|level| level.from + 10.0).reduce(f32::max);
            let now = state.trim().parse::<f32>().ok().filter(|value| value.is_finite());
            a.level_colors.push(MeterLevel { from: top.or(now).unwrap_or(0.0), color: SWATCHES[4] });
        }
        if ui.button("Colour for a state").on_hover_text("For words such as heating or unavailable").clicked() {
            add_state_color(&mut a.state_colors, state);
        }
    });
}

/// A text field with a right-click menu, for people who paste that way.
pub(super) fn text_field(ui: &mut Ui, field: TextEdit<'_>) -> egui::Response {
    let response = ui.add(field);
    response.context_menu(|menu| {
        let chosen = [
            ("Cut", ViewportCommand::RequestCut),
            ("Copy", ViewportCommand::RequestCopy),
            ("Paste", ViewportCommand::RequestPaste),
        ]
        .into_iter()
        .find_map(|(label, command)| menu.button(label).clicked().then_some(command));
        if let Some(command) = chosen {
            response.request_focus();
            menu.ctx().send_viewport_cmd(command);
            menu.close();
        }
    });
    response
}

pub(super) fn hash_of(value: &impl std::hash::Hash) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

pub fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
    for px in &image.pixels {
        let [r, g, b, _] = px.to_array();
        bytes.extend_from_slice(&[r, g, b, 255]);
    }
    image::save_buffer_with_format(
        path,
        &bytes,
        image.size[0] as u32,
        image.size[1] as u32,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .map_err(|e| e.to_string())
}

/// Whether Home Assistant may press a key: only the person chooses this.
pub(super) fn remote_choice(ui: &mut Ui, key: &mut DockKey) {
    use crate::model::RemoteAccess;
    heading(ui, "PRESSING FROM HOME ASSISTANT");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut key.remote, RemoteAccess::Auto, "Automatic")
            .on_hover_text("Allowed, unless the key opens programs, websites, shortcuts or types text");
        ui.selectable_value(&mut key.remote, RemoteAccess::Allow, "Always");
        ui.selectable_value(&mut key.remote, RemoteAccess::Deny, "Never");
    });
    let hint = match key.remote {
        RemoteAccess::Auto => "Its automations and dashboards can press it, unless it acts on this PC.",
        RemoteAccess::Allow => "Its automations and dashboards can press it, whatever it does.",
        RemoteAccess::Deny => "Only you can press it.",
    };
    ui.label(dim(hint).size(12.5));
}
