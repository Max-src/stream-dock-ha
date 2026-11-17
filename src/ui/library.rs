//! The list of actions on the right, and adding one to a key.

use super::*;

/// The name the icon set goes by in the window's fonts.
pub(super) const ICON_FONT: &str = "icons";

pub(super) const GLYPHS: [&str; 30] = [
    "⏵", "⏮", "⏭", "🔊", "🔉", "🔇", "🌐", "📁", "📄", "🖩", "⌨", "✉", "📷", "🎵", "🎬", "🎮", "💡",
    "⭐", "🔒", "🔍", "⚙", "🗑", "📋", "📌", "✂", "➡", "⬅", "⬆", "⬇", "❤",
];

pub(super) struct LibraryItem {
    pub(super) group: &'static str,
    pub(super) title: &'static str,
    pub(super) glyph: &'static str,
    pub(super) color: [u8; 3],
    pub(super) make: fn() -> Action,
}

pub(super) const LIBRARY: &[LibraryItem] = &[
    LibraryItem {
        group: "Open",
        title: "Program or file",
        glyph: "📁",
        color: [46, 104, 214],
        make: || Action::Open {
            target: String::new(),
            args: String::new(),
        },
    },
    LibraryItem {
        group: "Open",
        title: "Website",
        glyph: "🌐",
        color: [32, 138, 104],
        make: || Action::Website { url: String::new() },
    },
    LibraryItem {
        group: "Keyboard",
        title: "Press keys",
        glyph: "⌨",
        color: [196, 106, 44],
        make: || Action::Shortcut { keys: String::new() },
    },
    LibraryItem {
        group: "Keyboard",
        title: "Type text",
        glyph: "📄",
        color: [196, 106, 44],
        make: || Action::TypeText { text: String::new(), press_enter: false },
    },
    LibraryItem {
        group: "Media",
        title: "Play / pause",
        glyph: "⏵",
        color: [112, 78, 196],
        make: || Action::Media {
            key: MediaKey::PlayPause,
        },
    },
    LibraryItem {
        group: "Media",
        title: "Volume up",
        glyph: "🔊",
        color: [112, 78, 196],
        make: || Action::Media {
            key: MediaKey::VolumeUp,
        },
    },
    LibraryItem {
        group: "Media",
        title: "Volume down",
        glyph: "🔉",
        color: [112, 78, 196],
        make: || Action::Media {
            key: MediaKey::VolumeDown,
        },
    },
    LibraryItem {
        group: "Media",
        title: "Mute",
        glyph: "🔇",
        color: [112, 78, 196],
        make: || Action::Media { key: MediaKey::Mute },
    },
    LibraryItem {
        group: "Microphone",
        title: "Microphone on / off",
        glyph: "🎤",
        color: [112, 78, 196],
        make: || Action::Media { key: MediaKey::MicMute },
    },
    LibraryItem {
        group: "Microphone",
        title: "Microphone louder",
        glyph: "🎤",
        color: [112, 78, 196],
        make: || Action::Media { key: MediaKey::MicUp },
    },
    LibraryItem {
        group: "Microphone",
        title: "Microphone quieter",
        glyph: "🎤",
        color: [112, 78, 196],
        make: || Action::Media { key: MediaKey::MicDown },
    },
    LibraryItem {
        group: "Smart home",
        title: "Home Assistant entity",
        glyph: "🏠",
        color: [0, 0, 0],
        make: || Action::HomeAssistant(HaAction::default()),
    },
    LibraryItem {
        group: "Smart home",
        title: "Restart Home Assistant",
        glyph: "🔄",
        color: [176, 62, 46],
        make: || Action::HomeAssistant(HaAction::restart_home_assistant()),
    },
    LibraryItem {
        group: "Modes",
        title: "Turn on Mini Monitor",
        glyph: "🖥",
        color: [32, 138, 104],
        make: || Action::ToggleMiniMonitor,
    },
    LibraryItem {
        group: "Modes",
        title: "Brighter",
        glyph: "🔆",
        color: [32, 138, 104],
        make: || Action::AdjustBrightness { change: BRIGHTNESS_STEP },
    },
    LibraryItem {
        group: "Modes",
        title: "Dimmer",
        glyph: "🔅",
        color: [32, 138, 104],
        make: || Action::AdjustBrightness { change: -BRIGHTNESS_STEP },
    },
    LibraryItem {
        group: "Move around",
        title: "Next page",
        glyph: "➡",
        color: [70, 80, 100],
        make: || Action::NextPage,
    },
    LibraryItem {
        group: "Move around",
        title: "Previous page",
        glyph: "⬅",
        color: [70, 80, 100],
        make: || Action::PreviousPage,
    },
    LibraryItem {
        group: "Move around",
        title: "Go to page",
        glyph: "⬆",
        color: [70, 80, 100],
        make: || Action::GoToPage { page: 1 },
    },
    LibraryItem {
        group: "Move around",
        title: "Switch profile",
        glyph: "⚙",
        color: [70, 80, 100],
        make: || Action::SwitchProfile { name: String::new() },
    },
    LibraryItem {
        group: "Move around",
        title: "Next profile",
        glyph: "➡",
        color: [70, 80, 100],
        make: || Action::NextProfile,
    },
    LibraryItem {
        group: "Move around",
        title: "Previous profile",
        glyph: "⬅",
        color: [70, 80, 100],
        make: || Action::PreviousProfile,
    },
    LibraryItem {
        group: "Move around",
        title: "First profile",
        glyph: "⬆",
        color: [70, 80, 100],
        make: || Action::FirstProfile,
    },
    LibraryItem {
        group: "Timing",
        title: "Wait",
        glyph: "⏳",
        color: [70, 80, 100],
        make: || Action::Wait { ms: 1000 },
    },
];

/// The microphone's own icons, so a mute key can show when it's muted.
fn key_icon(action: &Action) -> Option<&'static str> {
    match action {
        Action::Media { key: MediaKey::MicMute } => Some(MIC_ICON),
        Action::Media { key: MediaKey::MicUp } => Some("mdi:microphone-plus"),
        Action::Media { key: MediaKey::MicDown } => Some("mdi:microphone-minus"),
        Action::HomeAssistant(binding) if binding.restarts_home_assistant() => Some("mdi:restart"),
        _ => None,
    }
}

/// The library's "Home Assistant entity", which asks which entity.
fn picks_an_entity(action: &Action) -> bool {
    matches!(action, Action::HomeAssistant(binding) if binding.entity.is_empty() && binding.service.is_empty())
}

pub(super) fn add_library_action(key: &mut DockKey, index: usize) {
    let item = &LIBRARY[index];
    // A Home Assistant key takes its text from its own title, and its symbol
    // from the entity, unless it has one of its own (Restart Home Assistant).
    let action = (item.make)();
    let from_ha = matches!(action, Action::HomeAssistant(_));
    if key.title.is_empty() && !from_ha {
        key.title = item.title.into();
    }
    if key.glyph.is_empty() && key.image.is_none() && !key.no_icon {
        if let Some(icon) = key_icon(&action) {
            key.glyph = icon.into();
        } else if !from_ha {
            key.glyph = item.glyph.into();
        }
    }
    if key.actions.is_empty() {
        key.color = item.color;
    }
    key.actions.push((item.make)());
}

impl DockApp {
    pub(super) fn library(&mut self, ui: &mut Ui, cfg: &mut Config) {
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let mut group = "";
            // Mini Monitor and brightness are for a pad's own screen, which a
            // virtual pad doesn't have.
            let has_screen = cfg.profile().pad.has_hardware();
            for (index, item) in LIBRARY.iter().enumerate().filter(|(_, item)| has_screen || item.group != "Modes") {
                if item.group != group {
                    group = item.group;
                    ui.add_space(4.0);
                    heading(ui, &group.to_uppercase());
                }
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 31.0), Sense::click_and_drag());
                if response.hovered() {
                    ui.painter().rect_filled(rect, 7, c_card());
                }
                let chip = Rect::from_min_size(rect.min + vec2(5.0, 3.5), Vec2::splat(24.0));
                ui.painter().rect_filled(chip, 6, rgb(item.color));
                ui.painter().text(
                    chip.center(),
                    Align2::CENTER_CENTER,
                    item.glyph,
                    FontId::proportional(14.0),
                    Color32::WHITE,
                );
                ui.painter().text(
                    rect.left_center() + vec2(38.0, 0.0),
                    Align2::LEFT_CENTER,
                    item.title,
                    FontId::proportional(14.0),
                    c_text(),
                );
                response.dnd_set_drag_payload(NewAction(index));
                if response.clicked() && picks_an_entity(&(item.make)()) {
                    self.browser = Some(EntityBrowser::new(BrowserTarget::NewKeys));
                } else if response.clicked() {
                    let profile = cfg.profile_mut();
                    let page = profile.active_page;
                    let target = self.selected.or_else(|| profile.first_free_cell(page));
                    let keys = &mut profile.pages[page].keys;
                    match target {
                        Some(i) => {
                            add_library_action(&mut keys[i], index);
                            self.selected = Some(i);
                            self.selected_tile = None;
                        }
                        None => self.shared.set_status("This page is full: add a page or clear a key"),
                    }
                }
            }
        });
    }
}
