//! Questions the window asks, file pickers, and what happens with a picked file.

use super::*;

impl DockApp {
    /// File pickers run on their own thread so a long look around the disk
    /// doesn't hold up the shortcuts; their answers arrive here.
    pub(super) fn apply_picked(&mut self, cfg: &mut Config) {
        while let Ok(picked) = self.picked_rx.try_recv() {
            let place = match &picked {
                Picked::Program(p, ..) | Picked::Image(p, _) | Picked::HaImage(p, ..) => *p,
                Picked::StartupPicture(path) => {
                    self.startup_candidate = Some(path.clone());
                    continue;
                }
                Picked::SaveProfile(name, path) => {
                    let Some(profile) = cfg.profiles.iter().find(|profile| &profile.name == name) else { continue };
                    let name = profile.name.clone();
                    let result = crate::model::export_profile(profile, path);
                    self.shared.set_status(match result {
                        Ok(()) => format!("Saved \"{name}\" to {}", path.display()),
                        Err(e) => format!("Couldn't save the profile: {e}"),
                    });
                    continue;
                }
                Picked::OpenProfile(path) => {
                    match crate::model::import_profile(path) {
                        Ok(mut profile) => {
                            profile.name = cfg.free_name(&profile.name);
                            // It joins the pad in the window if it's for that kind of pad.
                            profile.id = crate::model::new_id();
                            if cfg.profile().pad == profile.pad {
                                profile.pad_id = cfg.shown_pad_id();
                            }
                            self.shared.set_status(format!("Added the profile \"{}\"", profile.name));
                            let at = cfg.profiles.len();
                            cfg.profiles.push(profile);
                            cfg.repair();
                            cfg.choose_profile(at);
                            self.selected = None;
                            self.selected_tile = None;
                        }
                        Err(e) => self.shared.set_status(format!("Couldn't open it: {e}")),
                    }
                    continue;
                }
                Picked::BandPicture(tile, path) => {
                    if let Some(BandItem::Picture { path: chosen, .. }) = cfg.band.get_mut(*tile) {
                        *chosen = path.display().to_string();
                    }
                    continue;
                }
            };
            if cfg.profiles.get(place.profile).map(profile_stamp) != Some(place.stamp) {
                self.shared.set_status("The profiles changed while you were choosing, so nothing was changed");
                continue;
            }
            let key = cfg
                .profiles
                .get_mut(place.profile)
                .and_then(|p| p.pages.get_mut(place.page))
                .and_then(|p| p.keys.get_mut(place.key));
            let Some(key) = key else { continue };
            match picked {
                Picked::Program(_, step, path) => {
                    if let Some(Action::Open { target, .. }) = key.actions.get_mut(step) {
                        *target = path.display().to_string();
                    }
                }
                Picked::Image(_, path) => {
                    key.image = Some(path.display().to_string());
                    key.no_icon = false;
                }
                Picked::HaImage(_, step, on, path) => {
                    key.no_icon = false;
                    if let Some(Action::HomeAssistant(binding)) = key.actions.get_mut(step) {
                        let chosen = Some(path.display().to_string());
                        if on {
                            binding.on_image = chosen;
                        } else {
                            binding.off_image = chosen;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn save_profile_as(&self, ctx: &egui::Context, name: &str) {
        let (tx, ctx, owner) = (self.picked_tx.clone(), ctx.clone(), name.to_string());
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' })
            .collect();
        std::thread::spawn(move || {
            let chosen = rfd::FileDialog::new()
                .add_filter("Stream Dock HA profile", &["json"])
                .set_file_name(format!("{safe}.streamdockha.json"))
                .save_file();
            if let Some(path) = chosen {
                let _ = tx.send(Picked::SaveProfile(owner, path));
                ctx.request_repaint();
            }
        });
    }

    pub(super) fn open_profile_file(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.picked_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let chosen = rfd::FileDialog::new()
                .add_filter("Stream Dock HA profile", &["json"])
                .pick_file();
            if let Some(path) = chosen {
                let _ = tx.send(Picked::OpenProfile(path));
                ctx.request_repaint();
            }
        });
    }

    pub(super) fn pick_file(&self, ctx: &egui::Context, images_only: bool, answer: impl Fn(PathBuf) -> Picked + Send + 'static) {
        let (tx, ctx) = (self.picked_tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if images_only {
                dialog = dialog.add_filter("Pictures", &["png", "jpg", "jpeg", "webp"]);
            }
            if let Some(path) = dialog.pick_file() {
                let _ = tx.send(answer(path));
                ctx.request_repaint();
            }
        });
    }

    pub(super) fn startup_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = self.startup_candidate.clone() else { return };
        Modal::new(Id::NULL.with("startup")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Store this as the pad's startup picture?").strong());
            ui.label(dim(path.display().to_string()).size(12.5));
            ui.add_space(4.0);
            ui.label("This replaces the startup picture stored inside the pad, and StreamDock's own startup picture with it. Keep the pad plugged in while it's sent.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Store it").clicked() {
                    *lock(&self.shared.startup_picture) = Some(path.clone());
                    crate::pad::wake();
                    self.startup_candidate = None;
                }
                if ui.button("Keep the current one").clicked() {
                    self.startup_candidate = None;
                }
            });
        });
    }

    pub(super) fn confirm_dialog(&mut self, ctx: &egui::Context, cfg: &mut Config) {
        let Some(confirm) = self.confirm.clone() else { return };
        let question = match &confirm {
            Confirm::DeleteProfile => format!("Delete the profile \"{}\" and all its keys?", cfg.profile().name),
            Confirm::DeletePage => format!("Delete page {} and all its keys?", cfg.profile().active_page + 1),
            Confirm::DeleteKey(cell) => {
                let page = &cfg.profile().pages[cfg.profile().active_page];
                let title = page.keys.get(*cell).map_or(String::new(), |key| key.title.replace('\n', " "));
                if title.trim().is_empty() {
                    "Delete this key?".to_string()
                } else {
                    format!("Delete \"{}\"?", title.trim())
                }
            }
            Confirm::DeleteSelection => format!("Delete these {} keys?", self.selection(cfg).len()),
            Confirm::Quit => "Quit Stream Dock HA? Your pads' keys stop working until it starts again.".to_string(),
            Confirm::ReplaceKnob(knob, choice) => format!(
                "Use the {} for the {}? What its turns and press do now is replaced.",
                SMALL_KNOBS[*knob].to_lowercase(),
                choice.label().to_lowercase()
            ),
            Confirm::RemovePad(pad_id) => {
                let name = cfg.pad(pad_id).map_or(String::new(), |entry| entry.name.clone());
                let profiles = match cfg.profiles_of(pad_id).len() {
                    1 => "Its profile and all its keys go".to_string(),
                    count => format!("Its {count} profiles and all their keys go"),
                };
                format!("Delete the virtual pad \"{name}\"? {profiles} with it, and Home Assistant cards showing it will say it's gone.")
            }
        };
        let (yes, no) = match confirm {
            Confirm::Quit => ("Quit", "Keep running"),
            Confirm::ReplaceKnob(..) => ("Replace", "Keep it"),
            Confirm::RemovePad(_) => ("Delete", "Keep it"),
            _ => ("Delete", "Keep it"),
        };
        Modal::new(Id::NULL.with("confirm")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(question);
            ui.add_space(8.0);
            if matches!(confirm, Confirm::DeleteKey(_) | Confirm::Quit) {
                ui.checkbox(&mut self.ask_no_more, "Don't ask me again");
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                let yes = if matches!(confirm, Confirm::RemovePad(_)) { RichText::new(yes).color(c_warn()) } else { RichText::new(yes) };
                if ui.button(yes).clicked() {
                    match confirm.clone() {
                        Confirm::Quit => {
                            if self.ask_no_more {
                                cfg.ask_before_quit = false;
                            }
                            // Saved as the app quits, after this frame.
                            self.quitting = true;
                        }
                        Confirm::DeleteKey(cell) => {
                            if self.ask_no_more {
                                cfg.ask_before_delete = false;
                            }
                            if let Some(key) = cfg.profile_mut().active_keys_mut().get_mut(cell) {
                                *key = DockKey::default();
                            }
                        }
                        Confirm::DeleteSelection => self.delete_selection(cfg),
                        Confirm::DeleteProfile => {
                            cfg.remove_active_profile();
                        }
                        Confirm::DeletePage => {
                            let p = cfg.profile_mut();
                            p.pages.remove(p.active_page);
                        }
                        Confirm::RemovePad(pad_id) => {
                            cfg.remove_pad(&pad_id);
                            self.on_screen.retain(|open| *open != pad_id);
                        }
                        Confirm::ReplaceKnob(knob, choice) => {
                            self.confirm = None;
                            self.set_knob(cfg, knob, choice);
                            return;
                        }
                    }
                    cfg.repair();
                    self.selected = None;
                    self.recording = None;
                    self.confirm = None;
                }
                if ui.button(no).clicked() {
                    self.confirm = None;
                }
            });
        });
    }
}
