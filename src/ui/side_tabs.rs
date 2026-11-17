//! The Look, Monitor and Settings tabs on the right.

use super::*;


impl DockApp {
    pub(super) fn look_settings(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.label(RichText::new("This pad").size(15.0).strong());
        heading(ui, "BACKGROUND");
        let (mut face, mut clear) = cfg.shown_pad().map_or(([0, 0, 0], false), |entry| (entry.face, entry.face_clear));
        ui.horizontal(|ui| {
            for (name, color) in [("Black", [0, 0, 0]), ("White", [255, 255, 255])] {
                if ui.selectable_label(!clear && face == color, name).clicked() {
                    (face, clear) = (color, false);
                }
            }
            if ui.selectable_label(clear, "Transparent").on_hover_text("Only the keys show, on whatever is behind the pad").clicked() {
                clear = true;
            }
            if ui.color_edit_button_srgb(&mut face).on_hover_text("Any other colour").changed() {
                clear = false;
            }
        });
        if let Some(entry) = cfg.shown_pad_mut() {
            (entry.face, entry.face_clear) = (face, clear);
        }
        ui.label(dim("Here and on screen.").size(12.5));
        ui.add_space(16.0);
        ui.label(RichText::new("All pads").size(15.0).strong());
        heading(ui, "KEY SHAPE");
        let style = &mut cfg.style;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut style.shape, KeyShape::Square, "Square");
            ui.selectable_value(&mut style.shape, KeyShape::Rounded, "Rounded");
            ui.selectable_value(&mut style.shape, KeyShape::Circle, "Circle");
        });
        if style.shape == KeyShape::Rounded {
            ui.add(egui::Slider::new(&mut style.corner, 4..=36).text("Corners"));
        }
        ui.add_space(8.0);
        heading(ui, "TEXT ON KEYS");
        let current = render::installed_fonts()
            .iter()
            .find(|(_, file)| *file == style.font)
            .map_or("Built-in font", |(name, _)| name.as_str());
        ComboBox::from_id_salt("font").selected_text(current).width(220.0).height(340.0).show_ui(ui, |ui| {
            ui.set_min_width(240.0);
            text_field(ui, TextEdit::singleline(&mut self.font_search).hint_text("Search").desired_width(220.0));
            ui.selectable_value(&mut style.font, String::new(), "Built-in font");
            let search = self.font_search.to_lowercase();
            for (name, file) in render::installed_fonts() {
                if search.is_empty() || name.to_lowercase().contains(&search) {
                    ui.selectable_value(&mut style.font, file.clone(), name.as_str());
                }
            }
        });
        ui.add(egui::Slider::new(&mut style.size, 8..=24).text("Size"));
        ui.horizontal(|ui| {
            ui.checkbox(&mut style.bold, "Bold");
            ui.checkbox(&mut style.italic, "Italic");
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut style.place, TextPlace::Top, "Top");
            ui.selectable_value(&mut style.place, TextPlace::Middle, "Middle");
            ui.selectable_value(&mut style.place, TextPlace::Bottom, "Bottom");
        });
        ui.horizontal(|ui| {
            ui.label(dim("Colour").size(12.5));
            ui.color_edit_button_srgb(&mut style.color);
        });
        ui.label(dim("Long text shrinks to fit.").size(12.5));
        // Only the Stream Dock 293S has a band.
        if cfg.profile().pad == PadKind::Main {
            ui.add_space(8.0);
            heading(ui, "SIDE BAND");
            ui.checkbox(&mut cfg.band_icons, "Icons instead of names")
                .on_hover_text("For CPU, GPU, Memory and Home Assistant sensors. A name you typed for a tile stays.");
        }
    }

    pub(super) fn home_assistant_settings(&mut self, ui: &mut Ui, cfg: &mut Config) {
        text_field(ui, TextEdit::singleline(&mut cfg.ha_url).desired_width(f32::INFINITY).hint_text("Address, like http://homeassistant.local:8123"));
        let hint = if cfg.ha_token.is_empty() { "Long-lived access token" } else { "Saved. Paste a new token to replace it" };
        text_field(ui, TextEdit::singleline(&mut self.ha_token_input).password(true).desired_width(f32::INFINITY).hint_text(hint));
        if ui.button("Connect").clicked() {
            let token = self.ha_token_input.trim();
            if !token.is_empty() {
                match ha::protect(token) {
                    Some(stored) => cfg.ha_token = stored,
                    None => self.shared.set_status("Windows couldn't protect the token"),
                }
                self.ha_token_input.clear();
            }
            self.shared.save_now_with(cfg);
            self.shared.ha.reconnect();
        }
        let status = lock(&self.shared.ha.status).clone();
        let count = lock(&self.shared.ha.entities).len();
        let color = if matches!(status, ha::HaStatus::Connected(_)) { c_good() } else { c_dim() };
        let detail = if count > 0 { format!("{}, {count} entities", status.label()) } else { status.label() };
        ui.label(RichText::new(detail).color(color).size(12.5));
        if cfg.ha_token.is_empty() {
            ui.label(dim("Token: in Home Assistant, your profile › Security › Long-lived access tokens › Create token. It's stored encrypted.").size(12.5));
        }
    }

    /// The Stream Dock HA integration in Home Assistant: whether to use it,
    /// this PC's name there, and how it's doing.
    fn integration_settings(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.checkbox(&mut cfg.ha_integration, "Show this PC and its pads in Home Assistant")
            .on_hover_text("When the integration is installed in Home Assistant, this PC and its pads show there as devices, and on dashboard cards.");
        ui.add_enabled_ui(cfg.ha_integration, |ui| {
            ui.horizontal(|ui| {
                ui.label(dim("PC name").size(12.5));
                let windows = std::env::var("COMPUTERNAME").unwrap_or_default();
                text_field(ui, TextEdit::singleline(&mut cfg.pc_name).desired_width(170.0).hint_text(windows));
            });
        });
        let status = self.shared.bridge.status();
        let color = if status == crate::bridge::Status::Connected { c_good() } else { c_dim() };
        let connected = matches!(*lock(&self.shared.ha.status), ha::HaStatus::Connected(_));
        if cfg.ha_integration && connected {
            ui.label(RichText::new(status.label(&cfg.pc_name_or_default())).color(color).size(12.5));
        }
    }

    pub(super) fn settings(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.add_space(2.0);
        egui::CollapsingHeader::new(RichText::new("General").strong()).default_open(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(dim("Theme"));
                ui.selectable_value(&mut cfg.theme, ThemeChoice::Windows, "Windows");
                ui.selectable_value(&mut cfg.theme, ThemeChoice::Light, "Light");
                ui.selectable_value(&mut cfg.theme, ThemeChoice::Dark, "Dark");
            });
            ui.checkbox(&mut cfg.start_hidden, "Start without opening the window");
            ui.checkbox(&mut cfg.ask_before_delete, "Ask before deleting a key");
            ui.checkbox(&mut cfg.ask_before_quit, "Ask before quitting");
            let mut autostart = tray::autostart_enabled();
            if ui.checkbox(&mut autostart, "Start with Windows").changed() {
                if let Err(e) = tray::set_autostart(autostart) {
                    self.shared.set_status(e);
                }
            }
        });
        egui::CollapsingHeader::new(RichText::new("Pads").strong()).default_open(true).show(ui, |ui| self.pads_settings(ui, cfg));
        let connected = matches!(*lock(&self.shared.ha.status), ha::HaStatus::Connected(_));
        let title = if connected { "Home Assistant" } else { "Home Assistant (optional)" };
        let mut section = egui::CollapsingHeader::new(RichText::new(title).strong()).default_open(false);
        if std::mem::take(&mut self.open_home_assistant) {
            section = section.open(Some(true));
        }
        section.show(ui, |ui| {
            heading(ui, "CONNECTION");
            self.home_assistant_settings(ui, cfg);
            ui.add_space(10.0);
            heading(ui, "HOME ASSISTANT CAN");
            ui.checkbox(&mut cfg.ha_control, "Press keys, switch profiles and pages, and change keys")
                .on_hover_text("From automations, scripts and dashboard cards. It also hears your presses. See docs/home-assistant-control.md.");
            ui.add_enabled_ui(cfg.ha_control, |ui| {
                ui.checkbox(&mut cfg.ha_control_programs, "Add keys that open programs or press keys")
                    .on_hover_text("Anyone who can edit your automations could then act on this PC. Whether it may press such a key is set on each key, under More.");
            });
            ui.add_space(10.0);
            heading(ui, "INTEGRATION");
            self.integration_settings(ui, cfg);
            ui.add_space(10.0);
            heading(ui, "WHILE IT'S UNAVAILABLE");
            ui.radio_value(&mut cfg.ha_offline_label, false, "Keys keep their last look, dimmed, with a mark")
                .on_hover_text("While the app can't reach Home Assistant, or it's restarting");
            ui.radio_value(&mut cfg.ha_offline_label, true, "Keys say \"HA offline\"");
        });
        ui.add_space(10.0);
        ui.separator();
        if ui.button("Quit Stream Dock HA").clicked() {
            if cfg.ask_before_quit {
                self.ask_no_more = false;
                self.confirm = Some(Confirm::Quit);
            } else {
                // Quitting saves, which needs the settings this frame is holding.
                self.quitting = true;
            }
        }
    }

    /// Every pad in one place: how it's doing, its brightness, and the
    /// 293S's startup picture. A pad not added yet can be added or tried.
    pub(super) fn pads_settings(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.horizontal(|ui| {
            ui.label("Turn screens off");
            let before = cfg.screen_off_ms;
            ComboBox::from_id_salt("screen-off").selected_text(crate::screen_off::label_of(cfg.screen_off_ms)).show_ui(ui, |ui| {
                for (ms, label) in crate::screen_off::CHOICES {
                    ui.selectable_value(&mut cfg.screen_off_ms, ms, label);
                }
            });
            if cfg.screen_off_ms != before {
                // A new time counts from now, rather than turning a pad off at once.
                for kind in PadKind::HARDWARE {
                    crate::screen_off::touched(kind);
                }
                crate::pad::wake();
            }
        });
        ui.label(dim("After this long without a press. A press wakes the screen; Mini Monitor keeps it on.").size(12.5));
        if cfg.screen_off_ms > 0 {
            // Keeps the time left below up to date.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        }
        ui.add_space(8.0);
        let pads: Vec<String> = cfg.pads_in_use().map(|entry| entry.id.clone()).collect();
        for pad_id in pads {
            self.pad_settings(ui, cfg, &pad_id);
            ui.add_space(8.0);
        }
        if ui.button(format!("Add a {}…", PadKind::Dashboard.label().to_lowercase())).on_hover_text(pad_editor::VIRTUAL_HINT).clicked() {
            self.start_new_virtual_pad(cfg);
        }
        ui.add_space(8.0);
        if cfg.small_pad_index().is_none() {
            self.small_pad_offer(ui, cfg);
            ui.add_space(8.0);
        }
        heading(ui, "PADS ON SCREEN");
        ui.checkbox(&mut cfg.on_screen_mouse, "Mouse on Mini Monitor").on_hover_text(super::virtual_pad::MOUSE_HINT);
        ui.checkbox(&mut cfg.on_screen_on_top, "Stay on top of other windows");
        ui.label(dim("Open one with On screen at the top. Right-click it for these too.").size(12.5));
    }

    /// One pad in the Pads section: its name, which can be changed in place,
    /// how it's doing and its own settings.
    fn pad_settings(&mut self, ui: &mut Ui, cfg: &mut Config, pad_id: &str) {
        let Some(entry) = cfg.pad(pad_id).cloned() else { return };
        self.pad_name_line(ui, cfg, &entry);
        let status = match entry.kind {
            PadKind::Main => lock(&self.shared.pad_status).label(),
            PadKind::Akp03e => small_pad::status(),
            PadKind::Dashboard => {
                let count = cfg.profiles_of(pad_id).len();
                let size = cfg.pad_profile_of(pad_id).map_or(String::new(), |i| format!("{} × {} keys, ", cfg.profiles[i].cols, cfg.profiles[i].rows));
                format!("{size}{count} profile{}. On screen and in Home Assistant only", if count == 1 { "" } else { "s" })
            }
        };
        let model = if entry.name == entry.kind.label() { String::new() } else { format!("{}: ", entry.kind.label()) };
        ui.label(dim(format!("{model}{status}")).size(12.5));
        match entry.kind {
            PadKind::Main | PadKind::Akp03e => {
                self.firmware_line(ui, entry.kind);
                self.screen_off_line(ui, cfg, entry.kind);
                let Some(brightness) = cfg.pad_mut(pad_id).map(|entry| &mut entry.brightness) else { return };
                if ui.add(egui::Slider::new(brightness, 0..=100).suffix("%").text("Brightness")).changed() {
                    // A change of brightness should be seen, so it wakes the pad.
                    crate::screen_off::touched(entry.kind);
                }
            }
            PadKind::Dashboard => {}
        }
        match entry.kind {
            PadKind::Main => {
                if ui.button("Startup picture…").on_hover_text("Shown when the pad starts and no app is connected. It's saved inside the pad, so change it only now and then.").clicked() {
                    self.pick_file(ui.ctx(), true, Picked::StartupPicture);
                }
            }
            PadKind::Akp03e => {
                ui.label(dim("Startup picture: not supported yet.").size(12.5));
            }
            PadKind::Dashboard => self.dashboard_settings(ui, cfg, pad_id),
        }
    }

    /// A pad's name, with a way to change it where it is.
    fn pad_name_line(&mut self, ui: &mut Ui, cfg: &mut Config, entry: &crate::model::PadEntry) {
        let editing = self.renaming_pad.as_ref().is_some_and(|(id, _)| *id == entry.id);
        if !editing {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&entry.name).strong());
                if ui.small_button("Rename").clicked() {
                    self.renaming_pad = Some((entry.id.clone(), entry.name.clone()));
                }
            });
            return;
        }
        let Some((_, name)) = &mut self.renaming_pad else { return };
        let mut done = false;
        ui.horizontal(|ui| {
            let field = text_field(ui, TextEdit::singleline(name).desired_width(170.0));
            field.request_focus();
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter || ui.small_button("Save").clicked() {
                match cfg.rename_pad(&entry.id, name) {
                    Ok(()) => done = true,
                    Err(problem) => self.shared.set_status(problem),
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
        if done {
            self.renaming_pad = None;
        }
    }

    /// A virtual pad's size, and the window to change or delete it.
    fn dashboard_settings(&mut self, ui: &mut Ui, cfg: &mut Config, pad_id: &str) {
        ui.horizontal(|ui| {
            if ui.button("Edit…").on_hover_text("Its name and how many keys it has").clicked() {
                self.edit_virtual_pad(cfg, pad_id);
            }
            if ui.button(RichText::new("Delete…").color(c_warn())).on_hover_text("With its profiles and keys").clicked() {
                self.confirm = Some(Confirm::RemovePad(pad_id.to_string()));
            }
        });
    }

    /// Whether a pad's screen is on, and for how long, with a button to wake
    /// it or to turn it off now.
    fn firmware_line(&mut self, ui: &mut Ui, kind: PadKind) {
        let found = crate::firmware::found(kind);
        let reading = matches!(&found, Some(Err(why)) if why == crate::firmware::READING);
        ui.horizontal(|ui| {
            let button = ui.add_enabled(!reading, egui::Button::new("Firmware version").small());
            if button.on_hover_text("Reads it from the pad, as the StreamDock app does. Nothing on the pad is changed.").clicked() {
                let shared = self.shared.clone();
                crate::firmware::read_in_background(kind, move || shared.repaint());
            }
            match found {
                Some(Ok(version)) => {
                    ui.label(RichText::new(version).size(12.5));
                }
                Some(Err(why)) => {
                    ui.label(dim(why).size(12.5));
                }
                None => {}
            }
        });
    }

    fn screen_off_line(&mut self, ui: &mut Ui, cfg: &Config, kind: PadKind) {
        let Some(left) = crate::screen_off::time_left(cfg, kind) else {
            if cfg.screen_off_ms > 0 && cfg.monitor_on(kind) {
                ui.label(dim("Screen stays on while Mini Monitor is on").size(12.5));
            }
            return;
        };
        ui.horizontal(|ui| {
            if left.is_zero() {
                ui.label(dim("Screen off: a press on the pad wakes it").size(12.5));
                if ui.small_button("Wake it").clicked() {
                    crate::screen_off::touched(kind);
                    crate::pad::wake();
                    self.shared.repaint();
                }
            } else {
                ui.label(dim(format!("Screen goes off in {}", crate::screen_off::spoken(left))).size(12.5));
                if ui.small_button("Turn it off now").on_hover_text("To see how it looks; a press on the pad wakes it").clicked() {
                    crate::screen_off::sleep_now(cfg, kind);
                    crate::pad::wake();
                    self.shared.repaint();
                }
            }
        });
    }

    /// Offers the AKP03E before it's added. Each of its presses is in the
    /// pad log once it runs, with its code and cell.
    pub(super) fn small_pad_offer(&mut self, ui: &mut Ui, cfg: &mut Config) {
        let name = PadKind::Akp03e.label();
        ui.label(RichText::new(name).strong());
        ui.label(dim(format!("Runs an {name} next to your Stream Dock, with its own profiles.")).size(12.5));
        if ui.button("Use this pad").clicked() {
            self.use_small_pad(cfg);
        }
    }

    pub(super) fn monitor_settings(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ui.add_space(4.0);
        ui.label(RichText::new("Mini Monitor").size(16.0).strong());
        ui.label(dim("Your pad becomes an extra Windows screen.").size(12.5));
        ui.add_space(4.0);
        if !self.driver_settings(ui) {
            return;
        }
        if cfg.small_pad_index().is_some() {
            heading(ui, "WHICH PAD");
            let pads: Vec<(PadKind, String)> =
                cfg.pads_in_use().filter(|entry| entry.kind.has_mini_monitor()).map(|entry| (entry.kind, entry.name.clone())).collect();
            for (kind, name) in pads {
                ui.radio_value(&mut cfg.monitor_pad, kind, name);
            }
            ui.add_space(6.0);
        }
        let label = if cfg.mini_monitor { "Turn off" } else { "Turn on" };
        if ui.button(RichText::new(format!("  {label}  ")).size(15.0)).clicked() {
            cfg.mini_monitor = !cfg.mini_monitor;
        }
        ui.label(dim("Any press on that pad goes back to your keys.").size(12.5))
            .on_hover_text("To start it from a pad, add the Turn on Mini Monitor action to one of its keys.");
        ui.add_space(12.0);
        if cfg.monitor_pad == PadKind::Main {
            heading(ui, "WHERE THE PICTURE GOES");
            ui.radio_value(&mut cfg.monitor_area, MonitorArea::KeysOnly, "Keys only, band keeps its sensors");
            ui.radio_value(&mut cfg.monitor_area, MonitorArea::KeysAndBand, "Keys and band");
        }
        if cfg.monitor_pad == PadKind::Main && cfg.monitor_area == MonitorArea::KeysAndBand {
            ui.add_space(4.0);
            ui.label(dim("Dim the brighter side if they differ.").size(12.5));
            ui.horizontal(|ui| {
                ui.label(dim("Band").size(12.5));
                ui.add(egui::Slider::new(&mut cfg.band_balance, -60..=60).show_value(false));
                ui.label(dim("Keys").size(12.5));
            });
        }
        ui.add_space(12.0);
        heading(ui, "GAPS BETWEEN KEYS");
        ui.checkbox(&mut cfg.monitor_gaps, "Allow for the gaps");
        ui.label(dim("Lines stay straight; what falls under a gap is hidden.").size(12.5));
        ui.add_enabled_ui(cfg.monitor_gaps, |ui| {
            if ui.button("Adjust on the pad…").clicked() {
                self.adjusting = true;
                self.shared.alignment_pattern.store(true, Ordering::SeqCst);
                crate::pad::wake();
            }
        });
        ui.add_space(12.0);
        heading(ui, "ON SCREEN AND ON CARDS");
        ui.checkbox(&mut cfg.on_screen_seamless, "One picture across the gaps")
            .on_hover_text("Pads on screen and Home Assistant cards show it like a small monitor, with square edges. The real pad always has its gaps. Also in a pad on screen's right-click menu.");
        ui.add_space(12.0);
        heading(ui, "ON HOME ASSISTANT CARDS");
        // A label beside a value doesn't wrap: a long one here once made the
        // column wider than its place, so the pad's area covered its left side.
        ui.label("Pictures a second").on_hover_text("More is smoother but uses more network and CPU. Nothing is sent while no card shows the pad.");
        ui.add(egui::Slider::new(&mut cfg.ha_monitor_fps, 1..=15));
        ui.add_enabled_ui(cfg.ha_control, |ui| {
            ui.checkbox(&mut cfg.ha_mouse, "Cards can use the mouse")
                .on_hover_text("Clicks, drags and the wheel on a Mini Monitor card move this PC's real pointer and act on the extra display. Only Home Assistant administrators can.");
        });
        if !cfg.ha_control {
            ui.label(dim("The mouse needs Home Assistant to be allowed to press keys, in Settings.").size(12.5));
        } else if cfg.ha_mouse {
            ui.label(dim(crate::pointer_back::hint()).size(12.5));
        }
        ui.add_enabled_ui(cfg.ha_integration, |ui| {
            ui.checkbox(&mut cfg.ha_monitor_local, "Send over Home Assistant's local address")
                .on_hover_text("When the app reaches Home Assistant through another address, such as one on the internet, the pictures go straight over your home network instead. On an http address the token travels unencrypted on that network.");
        });
        if self.shared.bridge.frames_local() {
            ui.label(dim("Pictures are going over the local address.").size(12.5));
        }
    }

    /// Mini Monitor's display driver: fine if it's there, otherwise a button to
    /// install it. False while it's missing, so the rest of the tab waits.
    fn driver_settings(&mut self, ui: &mut Ui) -> bool {
        // Not known yet counts as there, so the tab doesn't flicker.
        if crate::driver::present_lately().unwrap_or(true) {
            return true;
        }
        ui.add_space(6.0);
        ui.label(RichText::new("It needs Parsec's free display driver.").color(c_warn()));
        use crate::driver::Step;
        let step = lock(&self.driver_step).clone();
        match step {
            Step::Idle | Step::Done => {
                if ui.button("Install the driver").on_hover_text("Downloads it from Parsec. Windows asks for administrator rights.").clicked() {
                    let step = self.driver_step.clone();
                    *lock(&step) = Step::Working(crate::driver::downloading(0, None));
                    let shared = self.shared.clone();
                    std::thread::spawn(move || {
                        let report = |words: String| {
                            *lock(&step) = Step::Working(words);
                            shared.repaint();
                        };
                        let result = crate::driver::install(&report);
                        *lock(&step) = match result {
                            Ok(()) => Step::Done,
                            Err(why) => Step::Failed(why),
                        };
                        shared.repaint();
                    });
                }
            }
            Step::Working(words) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(words);
                });
            }
            Step::Failed(why) => {
                ui.label(RichText::new(why).color(c_warn()).size(12.5));
                if ui.button("Download again").clicked() {
                    *lock(&self.driver_step) = Step::Idle;
                }
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        false
    }

    /// Lining up the gaps, with the test picture showing on the pad.
    pub(super) fn adjust_panel(&mut self, ui: &mut Ui, cfg: &mut Config) {
        heading(ui, "LINE UP THE PICTURE");
        let ready = match cfg.monitor_pad {
            PadKind::Main => lock(&self.shared.pad_status).is_working(),
            PadKind::Akp03e => small_pad::is_connected(),
            PadKind::Dashboard => false,
        };
        if ready {
            ui.label("Your pad is showing a test picture. Move the sliders until the diagonal lines run straight across the gaps and the circle looks round.");
        } else {
            ui.label(RichText::new("The pad isn't connected, so the test picture can't show yet.").color(c_warn()));
        }
        ui.add_space(8.0);
        ui.spacing_mut().slider_width = 320.0;
        let pad = cfg.monitor_pad;
        if let Some([across, down]) = cfg.gaps_mut(pad) {
            ui.add(egui::Slider::new(across, 0..=100).suffix("%").text("Gap across"));
            ui.add(egui::Slider::new(down, 0..=100).suffix("%").text("Gap down"));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Done").clicked() {
                self.shared.alignment_pattern.store(false, Ordering::SeqCst);
                crate::pad::wake();
            }
            if ui.button("Reset").clicked() {
                if let Some(gaps) = cfg.gaps_mut(pad) {
                    *gaps = pad.default_gaps();
                }
            }
        });
    }
}
