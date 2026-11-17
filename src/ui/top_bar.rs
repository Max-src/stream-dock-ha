//! The bar along the top: pads, profiles and the More menu.

use super::*;

impl DockApp {
    pub(super) fn top_bar(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.horizontal_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
            ui.painter().rect_filled(rect, 6, ACCENT);
            for c in 0..3 {
                for r in 0..2 {
                    let min = rect.min + vec2(4.0 + c as f32 * 6.5, 7.0 + r as f32 * 6.5);
                    ui.painter().rect_filled(Rect::from_min_size(min, vec2(5.0, 5.0)), 1, c_text());
                }
            }
            ui.label(RichText::new(APP_NAME).size(17.0).strong());
            ui.label(dim(&version_text()).size(12.0)).on_hover_text("The version running, and the change it was built from");
            ui.add_space(14.0);
            let in_use = cfg.small_pad_index().is_some();
            let shown = cfg.shown_pad_id();
            let pads: Vec<(String, String)> = cfg.pads_in_use().map(|entry| (entry.id.clone(), entry.name.clone())).collect();
            let hint = "Each pad has its own profiles, pages and keys, and all keep running whichever one is shown here.";
            let mut chosen = shown.clone();
            let virtual_ids: Vec<String> = cfg.pads.iter().filter(|entry| entry.kind == PadKind::Dashboard).map(|entry| entry.id.clone()).collect();
            if pads.len() <= 2 {
                for (pad_id, name) in &pads {
                    let tab = ui.selectable_label(shown == *pad_id, name).on_hover_text(hint);
                    if tab.clicked() {
                        chosen = pad_id.clone();
                    }
                    if virtual_ids.contains(pad_id) {
                        tab.context_menu(|ui| {
                            if ui.button("Edit this pad…").clicked() {
                                self.edit_virtual_pad(cfg, pad_id);
                                ui.close();
                            }
                            if ui.button("Delete this pad…").clicked() {
                                self.confirm = Some(Confirm::RemovePad(pad_id.clone()));
                                ui.close();
                            }
                        });
                    }
                }
            } else {
                // More pads than fit as tabs: one list, so the bar keeps its room.
                let name = cfg.shown_pad().map_or(String::new(), |entry| entry.name.clone());
                ComboBox::from_id_salt("pad").selected_text(name).width(150.0).show_ui(ui, |ui| {
                    for (pad_id, name) in &pads {
                        ui.selectable_value(&mut chosen, pad_id.clone(), name);
                    }
                }).response.on_hover_text(hint);
            }
            if chosen != shown {
                cfg.show_pad_id(&chosen);
                self.selected = None;
                self.selected_tile = None;
                self.recording = None;
                self.renaming = None;
            }
            let open = self.on_screen.contains(&shown);
            let toggle = ui.selectable_label(open, "On screen").on_hover_text(
                "This pad in a window of its own on your desktop. Click it as you'd press the pad, plugged in or not.",
            );
            if toggle.clicked() {
                self.toggle_on_screen(&shown);
            }
            if virtual_ids.contains(&shown) && ui.button("Edit pad").on_hover_text("Its name and how many keys it has, or delete it").clicked() {
                self.edit_virtual_pad(cfg, &shown);
            }
            ui.menu_button("Add a pad", |ui| {
                ui.set_max_width(270.0);
                if in_use {
                    ui.label(dim("Your AJAZZ AKP03E is already added."));
                } else if ui.button(PadKind::Akp03e.label()).clicked() {
                    self.use_small_pad(cfg);
                    ui.close();
                }
                if ui.button(format!("{}…", PadKind::Dashboard.label())).on_hover_text(pad_editor::VIRTUAL_HINT).clicked() {
                    self.start_new_virtual_pad(cfg);
                    ui.close();
                }
                ui.separator();
                ui.label(dim("Supported: Stream Dock 293S and AJAZZ AKP03E. Set one up before it's plugged in and try it with On screen.").size(12.5));
            });
            ui.add_space(12.0);
            ui.label(dim("Profile"));

            if let Some(name) = &mut self.renaming {
                let field = text_field(ui, TextEdit::singleline(name).desired_width(170.0));
                field.request_focus();
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if enter || ui.button("Save").clicked() {
                    let new = name.trim().to_string();
                    let active = cfg.active_profile;
                    let taken = cfg.profiles.iter().enumerate().any(|(i, p)| i != active && p.name == new);
                    if new.is_empty() || taken {
                        self.shared.set_status("That profile name is empty or already used");
                    } else {
                        let old = std::mem::replace(&mut cfg.profile_mut().name, new.clone());
                        cfg.rename_profile_links(&old, &new);
                        self.renaming = None;
                    }
                }
                if escape {
                    self.renaming = None;
                }
            } else {
                // One menu for the profile: the ones to switch to, then what can be done with it.
                let mut chosen = cfg.active_profile;
                let title = format!("{}  ⏷", cfg.profile().name);
                let menu = ui.menu_button(title, |ui| {
                    ui.set_min_width(220.0);
                    for i in cfg.profiles_of(&cfg.shown_pad_id()) {
                        if ui.selectable_label(i == cfg.active_profile, cfg.profiles[i].name.clone()).clicked() {
                            chosen = i;
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("New profile").clicked() {
                        cfg.add_profile();
                        self.selected = None;
                        ui.close();
                    }
                    if ui.button("Duplicate this profile").clicked() {
                        let mut copy = cfg.profile().clone();
                        copy.name = cfg.free_name(&format!("{} copy", copy.name));
                        copy.id = crate::model::new_id();
                        cfg.profiles.push(copy);
                        cfg.choose_profile(cfg.profiles.len() - 1);
                        self.selected = None;
                        ui.close();
                    }
                    if ui.button("Rename this profile").clicked() {
                        self.renaming = Some(cfg.profile().name.clone());
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Save this profile to a file…").clicked() {
                        self.save_profile_as(ui.ctx(), &cfg.profile().name);
                        ui.close();
                    }
                    if ui.button("Open a profile from a file…").clicked() {
                        self.open_profile_file(ui.ctx());
                        ui.close();
                    }
                    // The Stream Dock and virtual pads always keep one profile;
                    // the AKP03E's last one takes it out of use.
                    let own_profiles = cfg.profiles_of(&cfg.shown_pad_id()).len();
                    if cfg.profile().pad == PadKind::Akp03e || own_profiles > 1 {
                        ui.separator();
                        if ui.button(RichText::new("Delete this profile…").color(c_warn())).clicked() {
                            self.confirm = Some(Confirm::DeleteProfile);
                            ui.close();
                        }
                    }
                });
                menu.response.on_hover_text("Switch profile, or add, copy, rename, save or delete one");
                if chosen != cfg.active_profile {
                    cfg.choose_profile(chosen);
                    self.selected = None;
                    self.selected_tile = None;
                    self.recording = None;
                }
            }
        });
    }

    /// Starts using the AKP03E, and shows it ready to set up.
    pub(super) fn use_small_pad(&mut self, cfg: &mut Config) {
        cfg.add_small_pad(PadKind::Akp03e.label());
        self.selected = None;
        self.selected_tile = None;
        self.recording = None;
        self.renaming = None;
        self.tab = Tab::Actions;
    }

    /// Offers to set up an AKP03E that's plugged in and not in use yet.
    pub(super) fn pad_offer(&mut self, ui: &mut Ui, cfg: &mut Config) {
        if cfg.small_pad_index().is_some() || !small_pad::is_plugged_in() {
            return;
        }
        Frame::new().fill(c_panel()).corner_radius(8).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("An AJAZZ AKP03E is plugged in.");
                if ui.button("Set it up").clicked() {
                    self.use_small_pad(cfg);
                }
            });
        });
        ui.add_space(6.0);
    }
}

/// The version, with the commit it was built from when that's known.
pub(super) fn version_text() -> String {
    let commit = env!("STREAM_DOCK_HA_COMMIT");
    if commit.is_empty() {
        format!("v{}", env!("CARGO_PKG_VERSION"))
    } else {
        format!("v{} · {commit}", env!("CARGO_PKG_VERSION"))
    }
}
