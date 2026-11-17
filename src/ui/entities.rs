//! Choosing Home Assistant entities: the browser and the entity field.

use super::*;

pub(super) struct EntityBrowser {
    pub(super) target: BrowserTarget,
    pub(super) filter: ha::EntityFilter,
    pub(super) added: usize,
    pub(super) focus: bool,
}

impl EntityBrowser {
    pub(super) fn new(target: BrowserTarget) -> EntityBrowser {
        let domain = if matches!(target, BrowserTarget::Band(_)) { "sensor" } else { "" };
        EntityBrowser {
            target,
            filter: ha::EntityFilter { domain: domain.into(), ..Default::default() },
            added: 0,
            focus: true,
        }
    }
}

/// An entity's state with its unit, unless it has no value to measure.
pub(super) fn shown_state(entity: &ha::Entity) -> String {
    if matches!(entity.state.as_str(), "unavailable" | "unknown") {
        entity.state.clone()
    } else {
        format!("{} {}", entity.state, entity.unit).trim().to_string()
    }
}

/// Points a key at an entity, with what usually suits that kind of entity.
pub(super) fn point_at(binding: &mut HaAction, id: &str) {
    binding.entity = id.to_string();
    let services = ha::services_for(id);
    binding.service = services.first().map_or(String::new(), |s| s.to_string());
    let default_title = binding.title == HaAction::default().title || binding.title == "{{state}}{{unit}}";
    if default_title {
        binding.title = if services.is_empty() { "{{state}}{{unit}}".into() } else { HaAction::default().title };
    }
}

impl DockApp {
    /// Shows the entity in use, with a button to choose another.
    pub(super) fn entity_choice(&mut self, ui: &mut Ui, entity: &mut String, target: BrowserTarget) {
        let shared = self.shared.clone();
        let entities = lock(&shared.ha.entities);
        ui.horizontal(|ui| {
            let found = entities.get(entity.as_str());
            let label = match found {
                Some(e) if !e.name.is_empty() => e.name.clone(),
                _ if entity.is_empty() => "Nothing chosen yet".to_string(),
                _ => entity.clone(),
            };
            ui.label(RichText::new(label).strong());
            if let Some(e) = found {
                ui.label(dim(format!("now {}", shown_state(e))).size(12.5));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Choose…").clicked() {
                    self.browser = Some(EntityBrowser::new(target));
                }
            });
        });
        // The id under the friendly name, unless it's already what's shown.
        if entities.get(entity.as_str()).is_some_and(|e| !e.name.is_empty()) {
            ui.label(dim(entity.as_str()).size(12.5));
        }
    }

    /// The window for finding entities among thousands.
    pub(super) fn entity_browser(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        let Some(mut browser) = self.browser.take() else { return };
        let shared = self.shared.clone();
        let entities = lock(&shared.ha.entities);
        let areas = lock(&shared.ha.areas);
        let no_area = String::new();
        let area_of = |id: &str| areas.get(id).unwrap_or(&no_area);

        let mut types: BTreeMap<String, usize> = BTreeMap::new();
        let mut classes: BTreeMap<String, usize> = BTreeMap::new();
        let mut area_names: BTreeMap<String, usize> = BTreeMap::new();
        let mut all = 0;
        let mut results: Vec<(&String, &ha::Entity)> = Vec::new();
        let words = browser.filter.search_words();
        for (id, entity) in entities.iter() {
            let area = area_of(id);
            let words_match = browser.filter.matches_these_words(&words, id, entity, area);
            if words_match {
                all += 1;
                *types.entry(ha::domain_of(id).to_string()).or_default() += 1;
            }
            let of_type = browser.filter.domain.is_empty() || browser.filter.domain == ha::domain_of(id);
            if of_type {
                if !entity.device_class.is_empty() {
                    *classes.entry(entity.device_class.clone()).or_default() += 1;
                }
                if !area.is_empty() {
                    *area_names.entry(area.clone()).or_default() += 1;
                }
            }
            if of_type && words_match {
                results.push((id, entity));
            }
        }
        results.sort_by_cached_key(|(id, e)| (e.name.to_lowercase(), id.as_str()));

        let mut picked: Option<String> = None;
        let mut close = false;
        let mut open_settings = false;
        let modal = Modal::new(Id::NULL.with("entities")).show(ctx, |ui| {
            ui.set_width(780.0);
            ui.set_height(540.0);
            ui.horizontal(|ui| {
                let title = match browser.target {
                    BrowserTarget::NewKeys => "Add Home Assistant keys",
                    _ => "Choose an entity",
                };
                ui.label(RichText::new(title).size(18.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Done").clicked() {
                        close = true;
                    }
                    if browser.added > 0 {
                        let keys = if browser.added == 1 { "key" } else { "keys" };
                        ui.label(RichText::new(format!("{} {keys} added", browser.added)).color(c_good()));
                    }
                });
            });
            if entities.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Home Assistant isn't connected").size(16.0).strong());
                    ui.label(dim(lock(&shared.ha.status).label()));
                    ui.add_space(8.0);
                    if ui.button("Open Home Assistant settings").clicked() {
                        open_settings = true;
                        close = true;
                    }
                });
                return;
            }
            let search = text_field(ui, TextEdit::singleline(&mut browser.filter.words)
                .desired_width(f32::INFINITY)
                .hint_text("Search by name, entity ID, area or state"));
            if std::mem::take(&mut browser.focus) {
                search.request_focus();
            }
            ui.horizontal(|ui| {
                let shown = if browser.filter.class.is_empty() { "Any kind".to_string() } else { browser.filter.class.replace('_', " ") };
                ComboBox::from_id_salt("kind").selected_text(shown).width(170.0).height(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut browser.filter.class, String::new(), "Any kind");
                    for (class, count) in &classes {
                        ui.selectable_value(&mut browser.filter.class, class.clone(), format!("{}  ({count})", class.replace('_', " ")));
                    }
                });
                if !area_names.is_empty() {
                    let shown = if browser.filter.area.is_empty() { "Any area".to_string() } else { browser.filter.area.clone() };
                    ComboBox::from_id_salt("area").selected_text(shown).width(170.0).height(320.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut browser.filter.area, String::new(), "Any area");
                        for (area, count) in &area_names {
                            ui.selectable_value(&mut browser.filter.area, area.clone(), format!("{area}  ({count})"));
                        }
                    });
                }
                ui.checkbox(&mut browser.filter.hide_unavailable, "Hide unavailable");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(dim(format!("{} found", results.len())));
                });
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(170.0);
                    ScrollArea::vertical().id_salt("types").auto_shrink([false, false]).show(ui, |ui| {
                        let everything = browser.filter.domain.is_empty();
                        if ui.selectable_label(everything, format!("All types  ({all})")).clicked() {
                            browser.filter.domain.clear();
                            browser.filter.class.clear();
                        }
                        let mut labelled: Vec<(String, &String, &usize)> =
                            types.iter().map(|(domain, count)| (ha::domain_label(domain), domain, count)).collect();
                        labelled.sort();
                        for (label, domain, count) in labelled {
                            let chosen = browser.filter.domain == *domain;
                            if ui.selectable_label(chosen, format!("{label}  ({count})")).clicked() {
                                browser.filter.domain = domain.clone();
                                browser.filter.class.clear();
                            }
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    if results.is_empty() {
                        ui.add_space(30.0);
                        ui.label(dim("Nothing matches. Try fewer words or another type."));
                    }
                    let row_height = 42.0;
                    ScrollArea::vertical().id_salt("results").auto_shrink([false, false]).show_rows(
                        ui,
                        row_height,
                        results.len(),
                        |ui, range| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (id, entity) in &results[range] {
                                let size = vec2(ui.available_width(), row_height);
                                let (rect, row) = ui.allocate_exact_size(size, Sense::click());
                                if row.hovered() {
                                    ui.painter().rect_filled(rect, 6, c_card());
                                }
                                let painter = ui.painter_at(rect.shrink(4.0));
                                let name = if entity.name.is_empty() { id.as_str() } else { entity.name.as_str() };
                                let at = rect.left_top() + vec2(8.0, 12.0);
                                painter.text(at, Align2::LEFT_CENTER, name, FontId::proportional(14.0), c_text());
                                let area = area_of(id);
                                let detail = if area.is_empty() { id.to_string() } else { format!("{id}  ·  {area}") };
                                let at = rect.left_top() + vec2(8.0, 30.0);
                                painter.text(at, Align2::LEFT_CENTER, detail, FontId::proportional(11.5), c_dim());
                                let state = shown_state(entity);
                                let at = rect.right_center() - vec2(10.0, 0.0);
                                painter.text(at, Align2::RIGHT_CENTER, state.trim(), FontId::proportional(13.0), c_dim());
                                if row.clicked() {
                                    picked = Some((*id).clone());
                                }
                            }
                        },
                    );
                });
            });
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                picked = results.first().map(|(id, _)| (*id).clone());
            }
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(id) = picked {
            match browser.target {
                BrowserTarget::Step(place, step) => {
                    let action = cfg
                        .profiles
                        .get_mut(place.profile)
                        .and_then(|p| p.pages.get_mut(place.page))
                        .and_then(|p| p.keys.get_mut(place.key))
                        .and_then(|k| k.actions.get_mut(step));
                    if let Some(Action::HomeAssistant(binding)) = action {
                        point_at(binding, &id);
                    }
                    close = true;
                }
                BrowserTarget::Band(tile) => {
                    if let Some(BandItem::HomeAssistant { entity, .. }) = cfg.band.get_mut(tile) {
                        *entity = id;
                    }
                    close = true;
                }
                BrowserTarget::NewKeys => {
                    let profile = cfg.profile_mut();
                    let page = profile.active_page;
                    let free = profile.first_free_cell(page);
                    let keys = &mut profile.pages[page].keys;
                    match free {
                        Some(i) => {
                            let mut binding = HaAction::default();
                            point_at(&mut binding, &id);
                            keys[i] = DockKey {
                                color: [0, 0, 0],
                                actions: vec![Action::HomeAssistant(binding)],
                                ..DockKey::default()
                            };
                            self.selected = Some(i);
                            self.selected_tile = None;
                            browser.added += 1;
                        }
                        None => shared.set_status("This page is full: add a page, then add more keys"),
                    }
                }
            }
        }
        if open_settings {
            self.tab = Tab::Settings;
            self.open_home_assistant = true;
        }
        if !close {
            self.browser = Some(browser);
        }
    }
}
