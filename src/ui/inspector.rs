//! The panel under the pad: a key's settings, its steps, and a band tile's settings.

use super::*;
use crate::model::RemoteAccess;

/// What can go in a Home Assistant key's text.
const TEXT_HINT: &str = "Use {{friendly_name}}, {{state}}, {{unit}} or any attribute, such as {{brightness}}. New lines are kept. Full Home Assistant templates work too, such as {{ states('sensor.x') | round(1) }}.";

/// A choice of how far back a graph reaches, with `none` for no stretch.
fn history_choice(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, span: &mut Option<crate::model::HistorySpan>, none: &str) {
    let shown = span.map_or(none, |span| span.label());
    ComboBox::from_id_salt(id).selected_text(shown).width(150.0).show_ui(ui, |ui| {
        ui.selectable_value(span, None, none);
        for choice in crate::model::HistorySpan::ALL {
            ui.selectable_value(span, Some(choice), choice.label());
        }
    });
}

impl DockApp {
    pub(super) fn ha_editor(&mut self, ui: &mut Ui, a: &mut HaAction, place: Place, step: usize) {
        self.entity_choice(ui, &mut a.entity, BrowserTarget::Step(place, step));
        if a.service_only() {
            ui.label(dim("No entity: the service runs on its own.").size(11.5));
        }
        // The usual services in one list; any other is typed, once asked for.
        let services = ha::services_for(&a.entity);
        let other_id = Id::NULL.with(("other-service", place.key, step));
        let typed = !a.service.is_empty() && !services.contains(&a.service.as_str());
        let mut other = typed || ui.data(|data| data.get_temp::<bool>(other_id).unwrap_or(false));
        ui.horizontal(|ui| {
            ui.label(dim("When pressed").size(12.5));
            let shown = if other {
                "Other service…".to_string()
            } else if a.service.is_empty() {
                "Nothing".to_string()
            } else {
                a.service.clone()
            };
            ComboBox::from_id_salt(("service", step)).selected_text(shown).width(180.0).show_ui(ui, |ui| {
                if ui.selectable_label(!other && a.service.is_empty(), "Nothing").clicked() {
                    a.service.clear();
                    other = false;
                }
                for service in services {
                    if ui.selectable_label(!other && a.service == *service, *service).clicked() {
                        a.service = service.to_string();
                        other = false;
                    }
                }
                if ui.selectable_label(other, "Other service…").clicked() && !other {
                    a.service.clear();
                    other = true;
                }
            });
        });
        ui.data_mut(|data| data.insert_temp(other_id, other));
        if other {
            text_field(ui, TextEdit::singleline(&mut a.service).desired_width(f32::INFINITY).hint_text("domain.service, like script.turn_on"));
        }
        ui.checkbox(&mut a.confirm, "Press twice to confirm")
            .on_hover_text("The first press shows \"Press again\" for 3 seconds; only a second press runs it");
        let (state, on_or_off) = {
            let entities = lock(&self.shared.ha.entities);
            let entity = entities.get(&a.entity);
            (entity.map(|entity| entity.state.clone()), entity.and_then(|entity| entity.is_on()).is_some())
        };
        let is_number = state.as_deref().is_some_and(|state| state.trim().parse::<f32>().is_ok_and(f32::is_finite));
        let can_toggle = ha::can_toggle(&a.entity);
        // On and off settings only for what has an on and an off, such as a
        // light or a door sensor, or where some are already set.
        let on_off = can_toggle || on_or_off || a.uses_on_off();

        ui.add_space(4.0);
        ui.label(dim("Text").size(12.5)).on_hover_text(TEXT_HINT);
        text_field(ui, TextEdit::multiline(&mut a.title).desired_rows(1).desired_width(f32::INFINITY).hint_text("{{friendly_name}}"))
            .on_hover_text(TEXT_HINT);
        ui.label(dim("Home Assistant templates work too: {{ states('sensor.x') | round(1) }}").size(11.5));
        // Only for sensors that give a number, or where it's already set.
        if is_number || a.graph.is_some() {
            ui.horizontal(|ui| {
                ui.label(dim("Graph").size(12.5)).on_hover_text("The sensor's recent past from Home Assistant, drawn behind the text");
                history_choice(ui, ("key-graph", step), &mut a.graph, "Off");
            });
        }
        if on_off {
            ui.horizontal(|ui| {
                let half = (ui.available_width() - 8.0) / 2.0;
                text_field(ui, TextEdit::singleline(&mut a.on_title).desired_width(half).hint_text("Text when on (optional)"));
                text_field(ui, TextEdit::singleline(&mut a.off_title).desired_width(half).hint_text("Text when off (optional)"));
            });
            ui.horizontal_wrapped(|ui| {
                if can_toggle {
                    ui.checkbox(&mut a.show_switch, "Show a switch")
                        .on_hover_text("When the key has no icon or picture of its own");
                }
                for (label, on) in [("Picture when on…", true), ("Picture when off…", false)] {
                    if ui.button(label).clicked() {
                        self.pick_file(ui.ctx(), true, move |path| Picked::HaImage(place, step, on, path));
                    }
                }
                if (a.on_image.is_some() || a.off_image.is_some()) && ui.button("No pictures").clicked() {
                    a.on_image = None;
                    a.off_image = None;
                }
            });
        }

        ui.add_space(4.0);
        ui.label(dim("Colours").size(12.5));
        if on_off {
            state_colors(ui, "Otherwise the key keeps its own colour", [
                ("Background when on", &mut a.on_color, SWATCHES[4]),
                ("Background when off", &mut a.off_color, SWATCHES[7]),
            ]);
            if a.show_switch && can_toggle {
                state_colors(ui, "Otherwise the switch keeps its usual colour", [
                    ("Switch when on", &mut a.on_track, render::SWITCH_ON),
                    ("Switch when off", &mut a.off_track, render::SWITCH_OFF),
                ]);
            }
        }
        value_colors(ui, a, state.as_deref().unwrap_or(""));

        egui::CollapsingHeader::new(dim("More").size(12.5)).id_salt(("ha-more", step)).default_open(!a.data.trim().is_empty()).show(ui, |ui| {
            text_field(ui, TextEdit::singleline(&mut a.data)
                    .desired_width(f32::INFINITY)
                    .hint_text("Extra service data, as JSON"),
            );
        });
    }

    pub(super) fn band_editor(&mut self, ui: &mut Ui, cfg: &mut Config, tile: usize) {
        heading(ui, &format!("SIDE BAND, TILE {} OF 3", tile + 1));
        ui.label(dim("Shows information; it can't be pressed.").size(12.5));
        ui.add_space(4.0);
        let item = &mut cfg.band[tile];
        ComboBox::from_id_salt(("band-kind", tile))
            .selected_text(item.label())
            .width(240.0)
            .show_ui(ui, |ui| {
                for choice in BandItem::choices() {
                    let same = std::mem::discriminant(&choice) == std::mem::discriminant(item);
                    if ui.selectable_label(same, choice.label()).clicked() && !same {
                        // Levels mean something else on a sensor's own range.
                        if matches!(item, BandItem::HomeAssistant { .. }) != matches!(choice, BandItem::HomeAssistant { .. }) {
                            cfg.band_meters[tile] = Default::default();
                        }
                        *item = choice;
                    }
                }
            });
        match item {
            BandItem::HomeAssistant { entity, label } => {
                ui.set_max_width(460.0);
                self.entity_choice(ui, entity, BrowserTarget::Band(tile));
                text_field(ui, TextEdit::singleline(label).desired_width(240.0).hint_text("Name on the tile (optional)"));
            }
            BandItem::Text { text } => {
                text_field(ui, TextEdit::singleline(text).desired_width(240.0).hint_text("Short text"));
            }
            BandItem::Picture { path, fit } => {
                ui.horizontal(|ui| {
                    text_field(ui, TextEdit::singleline(path).desired_width(240.0).hint_text("A PNG or JPEG file"));
                    if ui.button("Browse…").clicked() {
                        self.pick_file(ui.ctx(), true, move |path| Picked::BandPicture(tile, path));
                    }
                });
                picture_fit_choice(ui, "band-fit", fit);
            }
            _ => {}
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Background");
            let own = cfg.band_backgrounds[tile];
            if ui.selectable_label(own.is_none(), "Dark").on_hover_text("The band's own dark grey").clicked() {
                cfg.band_backgrounds[tile] = None;
            }
            let mut color = own.unwrap_or(render::BAND_BACKGROUND);
            swatches(ui, &mut color);
            ui.color_edit_button_srgb(&mut color).on_hover_text("Any other background colour");
            if Some(color) != own && (own.is_some() || color != render::BAND_BACKGROUND) {
                cfg.band_backgrounds[tile] = Some(color);
            }
        });
        self.meter_editor(ui, cfg, tile);
        if let BandItem::HomeAssistant { entity, .. } = &cfg.band[tile] {
            let state = lock(&self.shared.ha.entities).get(entity).map(|found| found.state.clone());
            let rules = &mut cfg.band_meters[tile].states;
            ui.add_space(4.0);
            state_color_rows(ui, rules, false);
            let button = ui.button("Colour for a state").on_hover_text("Colours the value for words such as heating or unavailable");
            if button.clicked() {
                add_state_color(rules, state.as_deref().unwrap_or(""));
            }
        }
    }

    /// How a tile draws its measurement: the look, the colours and where each
    /// colour starts.
    pub(super) fn meter_editor(&mut self, ui: &mut Ui, cfg: &mut Config, tile: usize) {
        let is_sensor = matches!(cfg.band[tile], BandItem::HomeAssistant { .. });
        if !is_sensor && !matches!(cfg.band[tile], BandItem::Cpu | BandItem::Memory | BandItem::Gpu) {
            return;
        }
        ui.add_space(6.0);
        if is_sensor {
            ui.checkbox(&mut cfg.band_meters[tile].on_sensor, "Show a meter")
                .on_hover_text("For sensors that give a number, such as a temperature");
            if !cfg.band_meters[tile].on_sensor {
                return;
            }
            let meter = &mut cfg.band_meters[tile];
            ui.horizontal(|ui| {
                ui.label("Runs from");
                ui.add(egui::DragValue::new(&mut meter.low).speed(0.5).max_decimals(1));
                ui.label("to");
                ui.add(egui::DragValue::new(&mut meter.high).speed(0.5).max_decimals(1));
            });
        }

        let stats = *lock(&self.shared.stats);
        let history = lock(&self.shared.history).clone();
        let mut face = {
            let entities = lock(&self.shared.ha.entities);
            band::faces(cfg, stats, &entities, &history)[tile].clone()
        };
        // The little pictures need something to draw, even before a reading.
        face.percent = face.percent.or(Some(62));
        if face.levels.is_empty() {
            face.levels = cfg.band_meters[tile].percent_levels();
        }
        if face.history.is_empty() {
            face.history = history[tile].clone();
        }
        // Square, or the corners would show as black notches on a light panel.
        let style = KeyStyle { shape: KeyShape::Square, ..cfg.style.clone() };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for choice in MeterStyle::ALL {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::click());
                self.paint_face(ui, rect, &Face { meter: choice, ..face.clone() }, &style);
                let chosen = cfg.band_meters[tile].style == choice;
                let edge = if chosen {
                    Stroke::new(2.5, ACCENT)
                } else if response.hovered() {
                    Stroke::new(1.0, c_text())
                } else {
                    Stroke::new(1.0, c_dim())
                };
                ui.painter().rect_stroke(rect, 0, edge, StrokeKind::Outside);
                if response.on_hover_text(choice.label()).clicked() {
                    cfg.band_meters[tile].style = choice;
                }
            }
        });

        let meter = &mut cfg.band_meters[tile];
        if is_sensor && meter.style == MeterStyle::Graph {
            ui.horizontal(|ui| {
                ui.label("Graph covers").on_hover_text("Longer stretches come from Home Assistant's own records");
                history_choice(ui, ("band-period", tile), &mut meter.period, "Live, the last 3 minutes");
            });
        }
        ui.add_space(4.0);
        let named = meter.preset().map_or("Your own colours", |preset| METER_PRESETS[preset].0);
        ComboBox::from_id_salt(("meter-colors", tile))
            .selected_text(named)
            .width(300.0)
            .show_ui(ui, |ui| {
                for (preset, (name, _)) in METER_PRESETS.iter().enumerate() {
                    if ui.selectable_label(meter.preset() == Some(preset), *name).clicked() {
                        meter.use_preset(preset);
                    }
                }
            });
        let (unit, decimals) = if is_sensor { ("", 1) } else { ("%", 0) };
        let mut remove = None;
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            for level in 0..meter.levels.len() {
                let lowest = if level == 0 { meter.low } else { meter.levels[level - 1].from };
                let highest = meter.levels.get(level + 1).map_or(meter.high, |next| next.from);
                color_choice(ui, &mut meter.levels[level].color);
                if level == 0 {
                    ui.label(dim("to start with"));
                } else {
                    ui.label("from");
                    let start = egui::DragValue::new(&mut meter.levels[level].from)
                        .range(lowest..=highest.max(lowest))
                        .speed(0.5)
                        .max_decimals(decimals)
                        .suffix(unit);
                    ui.add(start);
                    if ui.small_button("×").on_hover_text("Remove this colour").clicked() {
                        remove = Some(level);
                    }
                }
                ui.add_space(8.0);
            }
            let can_add = meter.levels.len() < MAX_METER_LEVELS && meter.levels.last().is_some_and(|top| top.from < meter.high);
            if ui.add_enabled(can_add, egui::Button::new("Add a colour")).clicked() {
                meter.add_level();
            }
        });
        if let Some(level) = remove {
            meter.levels.remove(level);
        }
        meter.repair();
    }

    pub(super) fn run(&self, key: &DockKey, profile: usize) {
        actions::run_key(self.shared.clone(), Some(profile), key.title.clone(), key.actions.clone(), false);
    }

    pub(super) fn step_editor(
        &mut self,
        ui: &mut Ui,
        action: &mut Action,
        place: Place,
        step: usize,
        names: &[String],
        pages: usize,
    ) {
        match action {
            Action::Open { target, args } => {
                ui.horizontal(|ui| {
                    text_field(ui, TextEdit::singleline(target)
                            .desired_width(ui.available_width() - 80.0)
                            .hint_text("A program, file or folder"),
                    );
                    if ui.button("Browse…").clicked() {
                        self.pick_file(ui.ctx(), false, move |path| Picked::Program(place, step, path));
                    }
                });
                text_field(ui, TextEdit::singleline(args)
                        .desired_width(f32::INFINITY)
                        .hint_text("Extra options (optional)"),
                );
            }
            Action::Website { url } => {
                text_field(ui, TextEdit::singleline(url)
                        .desired_width(f32::INFINITY)
                        .hint_text("example.com"),
                );
            }
            Action::Shortcut { keys } => {
                ui.horizontal(|ui| {
                    text_field(ui, TextEdit::singleline(keys)
                            .desired_width(ui.available_width() - 80.0)
                            .hint_text("Ctrl+Shift+M"),
                    );
                    let target = Recording::Step(place.key, step);
                    self.record_button(ui, target);
                });
                if !keys.trim().is_empty() {
                    if let Err(e) = actions::parse_combo(keys) {
                        ui.label(RichText::new(e).color(c_warn()).size(12.5));
                    }
                }
            }
            Action::TypeText { text, press_enter } => {
                text_field(ui, TextEdit::multiline(text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("The text to type"),
                );
                ui.checkbox(press_enter, "Press Enter afterwards");
            }
            Action::Media { key } => {
                ComboBox::from_id_salt(("media", step))
                    .selected_text(key.label())
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        // A microphone step offers the microphone's choices, a media step the rest.
                        let microphone = key.is_microphone();
                        for k in MediaKey::ALL.into_iter().filter(|k| k.is_microphone() == microphone) {
                            ui.selectable_value(key, k, k.label());
                        }
                    });
                if key.is_microphone() {
                    let hint = match key {
                        MediaKey::MicMute => "Windows' default microphone. The key shows when it's muted.",
                        _ => "Windows' default microphone, 2% a step. 0 mutes it.",
                    };
                    ui.label(dim(hint).size(12.5));
                }
            }
            Action::Wait { ms } => {
                ui.horizontal(|ui| {
                    ui.add(DragValue::new(ms).range(0..=60_000).speed(10.0).suffix(" ms"));
                    ui.label(dim("1000 ms is one second"));
                });
            }
            Action::GoToPage { page } => {
                ui.add(DragValue::new(page).range(1..=pages.max(1)).prefix("Page "));
            }
            Action::SwitchProfile { name } => {
                let shown = if name.is_empty() { "Choose a profile".to_string() } else { name.clone() };
                ComboBox::from_id_salt(("profile-step", step))
                    .selected_text(shown)
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for n in names {
                            ui.selectable_value(name, n.clone(), n.clone());
                        }
                    });
            }
            Action::AdjustBrightness { change } => {
                ui.add(DragValue::new(change).range(-100..=100).suffix(" %"));
            }
            Action::HomeAssistant(binding) => self.ha_editor(ui, binding, place, step),
            Action::NextPage
            | Action::PreviousPage
            | Action::NextProfile
            | Action::PreviousProfile
            | Action::FirstProfile
            | Action::ToggleMiniMonitor => {}
        }
    }

    pub(super) fn record_button(&mut self, ui: &mut Ui, target: Recording) {
        let active = self.recording == Some(target);
        let text = if active { "Press…" } else { "Record" };
        if ui.selectable_label(active, text).on_hover_text("Press the keys, or Esc to cancel").clicked() {
            if active {
                self.recording = None;
                recorder::stop();
            } else {
                recorder::stop();
                self.recording = Some(target);
                recorder::start(matches!(target, Recording::Step(..)));
            }
        }
    }

    pub(super) fn inspector(&mut self, ui: &mut Ui, cfg: &mut Config) {
        if self.adjusting {
            self.adjust_panel(ui, cfg);
            return;
        }
        if let Some(tile) = self.selected_tile {
            // A sensor tile's settings run past the bottom of a small window.
            ScrollArea::vertical().id_salt("band-settings").auto_shrink([false, false]).show(ui, |ui| self.band_editor(ui, cfg, tile));
            return;
        }
        if self.selection(cfg).len() > 1 {
            self.selection_panel(ui, cfg);
            return;
        }
        let Some(sel) = self.selected else {
            let hint = match cfg.profile().pad {
                PadKind::Main => "Click a key to edit it, then click or drag an action onto it. Double-click a key to try it. Click a band tile to choose what it shows.",
                PadKind::Akp03e => "Click a key, button or knob to set it up, then click or drag an action onto it. On a knob, click its left side for turning left, its right side for turning right, or its middle for pressing. Press a control on the pad and it lights up here.",
                PadKind::Dashboard => "Click a key to edit it, then click or drag an action onto it. Double-click a key to try it. Open it with On screen to press it like a pad.",
            };
            ui.centered_and_justified(|ui| {
                ui.label(dim(hint));
            });
            return;
        };
        let names: Vec<String> = cfg.profiles.iter().map(|p| p.name.clone()).collect();
        let pi = cfg.active_profile;
        let pg = cfg.profile().active_page;
        let pages = cfg.profile().pages.len();
        let place = Place {
            profile: pi,
            page: pg,
            key: sel,
            stamp: profile_stamp(&cfg.profiles[pi]),
        };
        let errors = lock(&self.shared.hotkey_errors).clone();
        let ha_control = cfg.ha_control;
        let small = cfg.profile().pad == PadKind::Akp03e;
        let shortcuts = cfg.profile().pad.has_shortcuts();
        let has_screen = cfg.profile().pad.cell_has_screen(sel);
        let largest = cfg.profile().largest_span(sel);
        let knob_now = small_knob_of_cell(sel).filter(|_| small).map(|knob| KnobUse::of(&cfg.profiles[pi].pages[pg].keys, knob));
        let mut knob_choice: Option<(usize, KnobUse)> = None;
        let key = &mut cfg.profiles[pi].pages[pg].keys[sel];

        ui.horizontal_top(|ui| {
            // Some keys have more settings than fit, so this column scrolls.
            ScrollArea::vertical().id_salt("key-settings").auto_shrink([true, false]).show(ui, |ui| ui.vertical(|ui| {
                ui.set_width(300.0);
                if let Some(knob) = small_knob_of_cell(sel).filter(|_| small) {
                    heading(ui, &SMALL_KNOBS[knob].to_uppercase());
                    ui.label(dim("Use the whole knob for").size(12.5));
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        for choice in KnobUse::ALL {
                            let current = knob_now.flatten() == Some(choice);
                            if ui.selectable_label(current, choice.label()).on_hover_text(choice.hint()).clicked() && !current {
                                knob_choice = Some((knob, choice));
                            }
                        }
                    });
                    ui.add_space(3.0);
                    ui.label(dim("Or set up each part on its own:").size(12.5));
                    ui.horizontal(|ui| {
                        for (cell, part) in small_knob_cells(knob).into_iter().zip(["Turn left", "Turn right", "Press"]) {
                            if ui.selectable_label(cell == sel, part).clicked() {
                                self.selected = Some(cell);
                                self.recording = None;
                            }
                        }
                    });
                    ui.add_space(3.0);
                } else {
                    heading(ui, "THIS KEY");
                }
                if key.camera().is_some() && largest != [1, 1] {
                    ui.horizontal(|ui| {
                        ui.label(dim("Camera size").size(12.5));
                        ui.add(DragValue::new(&mut key.span[0]).range(1..=largest[0]).suffix(" across"));
                        ui.add(DragValue::new(&mut key.span[1]).range(1..=largest[1]).suffix(" down"));
                    });
                    ui.label(dim("Or drag its blue corner. Keys under it are kept.").size(12.5));
                    ui.checkbox(&mut key.camera_gaps, "Allow for the gaps between keys").on_hover_text(
                        "On: lines stay straight, and what falls under a gap is hidden, as in Mini Monitor. Off: nothing is hidden, but the picture jumps at each gap. The size of the gaps is set in the Monitor tab.",
                    );
                    ui.add_space(3.0);
                }
                ui.horizontal(|ui| {
                    // Titles can have line breaks, which a one-line field would show as boxes.
                    text_field(ui, TextEdit::multiline(&mut key.title).desired_rows(1).desired_width(190.0).hint_text("Title"));
                    if ui.button("Try it").on_hover_text("Runs its actions now, as a press would").clicked() {
                        self.run(key, pi);
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let from_entity = key.actions.iter().any(|a| matches!(a, Action::HomeAssistant(_)));
                    let own = key.image.is_some() || !key.glyph.is_empty();
                    if from_entity
                        && ui
                            .selectable_label(!own && !key.no_icon, "Automatic")
                            .on_hover_text("The switch, or the icon this entity has in Home Assistant")
                            .clicked()
                    {
                        key.image = None;
                        key.glyph.clear();
                        key.no_icon = false;
                    }
                    icon_choice(ui, &mut self.icon_search, key);
                    if ui.selectable_label(key.image.is_some(), "Picture…").clicked() {
                        self.pick_file(ui.ctx(), true, move |path| Picked::Image(place, path));
                    }
                    let none = key.no_icon || (!from_entity && !own);
                    if ui.selectable_label(none, "No icon").on_hover_text("Only the text on this key").clicked() {
                        key.image = None;
                        key.glyph.clear();
                        key.no_icon = true;
                    }
                });
                let state_pictures = key.actions.iter().any(|action| {
                    matches!(action, Action::HomeAssistant(a) if a.on_image.is_some() || a.off_image.is_some())
                });
                if !key.no_icon && (key.image.is_some() || state_pictures || key.camera().is_some()) {
                    picture_fit_choice(ui, "key-fit", &mut key.picture_fit);
                }
                ui.horizontal(|ui| {
                    swatches(ui, &mut key.color);
                    ui.color_edit_button_srgb(&mut key.color).on_hover_text("Any other background colour");
                });
                ui.add_space(3.0);
                if !shortcuts && !has_screen {
                    ui.label(dim("No screen: this look only shows here.").size(12.5));
                }
                // Home Assistant may not press a key that acts on this PC unless
                // it's allowed, which is worth seeing without opening More.
                let blocked = ha_control && key.remote == RemoteAccess::Auto && key.actions.iter().any(crate::remote::acts_on_this_pc);
                if blocked {
                    ui.label(dim("Home Assistant can't press this key, as it acts on this PC. Change that under More.").size(12.5));
                }
                // What most keys don't need, folded away unless it's in use.
                let in_use = !key.hotkey.is_empty() || key.remote != RemoteAccess::Auto || !errors.is_empty();
                egui::CollapsingHeader::new(dim("More for this key").size(12.5))
                    .id_salt(("key-more", sel))
                    .default_open(in_use)
                    .show(ui, |ui| {
                        if shortcuts {
                            ui.label(dim("Keyboard shortcut").size(12.5)).on_hover_text("Runs this key from anywhere, even with the window closed");
                            ui.horizontal(|ui| {
                                if key.hotkey.is_empty() {
                                    ui.label(dim("No shortcut"));
                                } else {
                                    text_field(ui, TextEdit::singleline(&mut key.hotkey).desired_width(130.0));
                                }
                                self.record_button(ui, Recording::Hotkey(sel));
                                if !key.hotkey.is_empty() && ui.button("✖").on_hover_text("Remove the shortcut").clicked() {
                                    key.hotkey.clear();
                                }
                            });
                            for e in &errors {
                                ui.label(RichText::new(e).color(c_warn()).size(12.5));
                            }
                            ui.add_space(4.0);
                        }
                        remote_choice(ui, key);
                    });
            }));

            ui.separator();

            ui.vertical(|ui| {
                heading(ui, "ACTIONS, IN ORDER");
                if key.actions.is_empty() {
                    ui.label(dim("None yet. Click or drag one from the list on the right."));
                }
                let mut change: Option<(usize, i32)> = None;
                let count = key.actions.len();
                ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    for step in 0..count {
                        Frame::new()
                            .fill(c_card())
                            .corner_radius(8)
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(key.actions[step].label()).strong());
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.spacing_mut().button_padding = vec2(6.0, 2.0);
                                        if ui.small_button("✖").on_hover_text("Remove").clicked() {
                                            change = Some((step, 0));
                                        }
                                        if step + 1 < count && ui.small_button("⬇").clicked() {
                                            change = Some((step, 1));
                                        }
                                        if step > 0 && ui.small_button("⬆").clicked() {
                                            change = Some((step, -1));
                                        }
                                    });
                                });
                                self.step_editor(ui, &mut key.actions[step], place, step, &names, pages);
                            });
                    }
                });
                match change {
                    Some((step, 0)) => {
                        key.actions.remove(step);
                        self.recording = None;
                    }
                    Some((step, 1)) => key.actions.swap(step, step + 1),
                    Some((step, _)) => key.actions.swap(step, step - 1),
                    None => {}
                }
            });
        });
        if let Some((knob, choice)) = knob_choice {
            // A knob set up by hand asks first; one of these uses is just swapped.
            let page = &cfg.profiles[pi].pages[pg];
            let own = knob_now.flatten().is_none()
                && small_knob_cells(knob).iter().any(|cell| page.keys.get(*cell).is_some_and(|key| !key.actions.is_empty()));
            if own {
                self.confirm = Some(Confirm::ReplaceKnob(knob, choice));
            } else {
                self.set_knob(cfg, knob, choice);
            }
        }
    }

    /// Sets a whole knob of the page on show to one use.
    pub(super) fn set_knob(&mut self, cfg: &mut Config, knob: usize, choice: KnobUse) {
        let profile = cfg.profile_mut();
        let page = profile.active_page;
        profile.pages[page].set_knob(knob, choice);
        self.recording = None;
        self.shared.set_status(format!("The {} now controls the {}", SMALL_KNOBS[knob].to_lowercase(), choice.label().to_lowercase()));
    }

    /// Puts the cut or copied key into a key. Only the 293S has shortcuts.
    /// Ctrl+X, C, V, D and A, Delete and Esc on the chosen keys, unless a
    /// text field is in use.
    pub(super) fn key_shortcuts(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        if ctx.egui_wants_keyboard_input() || self.recording.is_some() || self.confirm.is_some() {
            return;
        }
        let (cut, copy, paste, duplicate, all, delete, escape) = ctx.input(|input| {
            let ctrl = input.modifiers.command;
            let pressed = |key: egui::Key| ctrl && input.key_pressed(key);
            let event = |wanted: &dyn Fn(&egui::Event) -> bool| input.events.iter().any(wanted);
            (
                pressed(egui::Key::X) || event(&|e| matches!(e, egui::Event::Cut)),
                pressed(egui::Key::C) || event(&|e| matches!(e, egui::Event::Copy)),
                pressed(egui::Key::V) || event(&|e| matches!(e, egui::Event::Paste(_))),
                pressed(egui::Key::D),
                pressed(egui::Key::A),
                input.modifiers.is_none() && (input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if all {
            self.select_all(cfg);
            return;
        }
        let Some(first) = self.selection(cfg).into_iter().min() else { return };
        let command = if delete {
            Some(KeyCommand::Delete)
        } else if cut {
            Some(KeyCommand::Cut)
        } else if copy {
            Some(KeyCommand::Copy)
        } else if paste {
            Some(KeyCommand::Paste)
        } else if duplicate {
            Some(KeyCommand::Duplicate)
        } else {
            None
        };
        if let Some(command) = command {
            self.key_command(cfg, first, command);
        } else if escape && !self.many.is_empty() {
            self.many.clear();
        }
    }
}
