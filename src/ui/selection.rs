//! Choosing several keys at once, and cutting, copying, pasting, duplicating
//! and deleting them together. One key on its own is the same as a group of one.

use super::*;

/// What can be done to the chosen keys, from a menu or a shortcut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum KeyCommand {
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
}

/// The cells in the rectangle between two cells, row by row.
pub(super) fn cells_between(cols: usize, a: usize, b: usize) -> Vec<usize> {
    let (top, bottom) = ((a / cols).min(b / cols), (a / cols).max(b / cols));
    let (left, right) = ((a % cols).min(b % cols), (a % cols).max(b % cols));
    (top..=bottom).flat_map(|row| (left..=right).map(move |column| row * cols + column)).collect()
}

/// The filled keys among these cells, each with its row and column from the
/// top left of the group, so a paste keeps their layout.
pub(super) fn clip_of(keys: &[DockKey], cells: &[usize], cols: usize) -> Vec<(usize, usize, DockKey)> {
    let filled: Vec<usize> = cells.iter().copied().filter(|cell| keys.get(*cell).is_some_and(|key| !key.is_empty())).collect();
    let top = filled.iter().map(|cell| cell / cols).min().unwrap_or(0);
    let left = filled.iter().map(|cell| cell % cols).min().unwrap_or(0);
    filled.iter().map(|cell| (cell / cols - top, cell % cols - left, keys[*cell].clone())).collect()
}

/// Where each copied key lands when the group is pasted with its top left on
/// a cell: (which copied key, which cell). Keys that would fall off the pad
/// are left out.
pub(super) fn paste_cells(clip: &[(usize, usize, DockKey)], at: usize, rows: usize, cols: usize) -> Vec<(usize, usize)> {
    let (row, column) = (at / cols, at % cols);
    clip.iter()
        .enumerate()
        .filter_map(|(index, (down, across, _))| {
            let (r, c) = (row + down, column + across);
            (r < rows && c < cols).then_some((index, r * cols + c))
        })
        .collect()
}

impl DockApp {
    /// The keys chosen now, in cell order: several, one, or none.
    pub(super) fn selection(&self, cfg: &Config) -> Vec<usize> {
        let Some(chosen) = self.selected else { return Vec::new() };
        let here = (cfg.active_profile, cfg.profile().active_page);
        if self.many_at == here && self.many.contains(&chosen) {
            self.many.clone()
        } else {
            vec![chosen]
        }
    }

    /// Makes these cells the chosen keys. A camera stretched over several
    /// keys counts once, by its own key.
    fn choose(&mut self, cfg: &Config, cells: Vec<usize>, focus: Option<usize>) {
        let profile = cfg.profile();
        let page = profile.active_page;
        let mut owners: Vec<usize> = cells.into_iter().map(|cell| profile.owner_of(page, cell)).collect();
        owners.sort_unstable();
        owners.dedup();
        self.selected = focus.filter(|cell| owners.contains(cell)).or_else(|| owners.first().copied());
        self.many_at = (cfg.active_profile, page);
        self.many = if owners.len() > 1 { owners } else { Vec::new() };
        self.selected_tile = None;
        self.recording = None;
    }

    /// A click on a key. On its own it chooses just that key, with Ctrl it
    /// adds or removes it, and with Shift it takes the rectangle from the key
    /// chosen before.
    pub(super) fn click_key(&mut self, cfg: &Config, cell: usize, modifiers: egui::Modifiers) {
        let cols = cfg.profile().cols;
        let mut chosen = self.selection(cfg);
        if modifiers.shift {
            let anchor = self.selected.unwrap_or(cell);
            let range = cells_between(cols, anchor, cell);
            if modifiers.command {
                chosen.extend(range);
            } else {
                chosen = range;
            }
            self.choose(cfg, chosen, Some(anchor));
        } else if modifiers.command {
            if let Some(at) = chosen.iter().position(|c| *c == cell) {
                chosen.remove(at);
                let focus = chosen.first().copied();
                self.choose(cfg, chosen, focus);
            } else {
                chosen.push(cell);
                self.choose(cfg, chosen, Some(cell));
            }
        } else {
            self.choose(cfg, vec![cell], Some(cell));
        }
    }

    /// The keys inside a box drawn on the pad. With Ctrl or Shift held, they
    /// join the keys already chosen.
    pub(super) fn box_select(&mut self, cfg: &Config, cells: Vec<usize>, add: bool) {
        if cells.is_empty() {
            return;
        }
        let mut chosen = if add { self.selection(cfg) } else { Vec::new() };
        chosen.extend(cells);
        self.choose(cfg, chosen, None);
    }

    /// Every key of the page, for Ctrl+A.
    pub(super) fn select_all(&mut self, cfg: &Config) {
        let profile = cfg.profile();
        let cells = (0..profile.key_count()).filter(|cell| profile.pad.cell_has_screen(*cell)).collect();
        self.choose(cfg, cells, None);
    }

    /// Does a command to the chosen keys. A key outside the group that was
    /// right-clicked is chosen on its own first.
    pub(super) fn key_command(&mut self, cfg: &mut Config, cell: usize, command: KeyCommand) {
        if !self.selection(cfg).contains(&cell) {
            self.choose(cfg, vec![cell], Some(cell));
        }
        match command {
            KeyCommand::Cut | KeyCommand::Copy => self.copy_selection(cfg, command == KeyCommand::Cut),
            KeyCommand::Paste => {
                let at = self.selection(cfg).into_iter().min().unwrap_or(cell);
                self.paste_at(cfg, at);
            }
            KeyCommand::Duplicate => self.duplicate_selection(cfg),
            KeyCommand::Delete => {
                let chosen = self.selection(cfg);
                let keys = &cfg.profile().pages[cfg.profile().active_page].keys;
                if chosen.iter().all(|cell| keys.get(*cell).is_none_or(DockKey::is_empty)) {
                    return;
                }
                if !cfg.ask_before_delete {
                    self.delete_selection(cfg);
                } else if chosen.len() > 1 {
                    self.confirm = Some(Confirm::DeleteSelection);
                } else {
                    self.ask_no_more = false;
                    self.confirm = Some(Confirm::DeleteKey(chosen[0]));
                }
            }
        }
    }

    /// Copies the chosen keys, and with `cut` empties their cells.
    pub(super) fn copy_selection(&mut self, cfg: &mut Config, cut: bool) {
        let chosen = self.selection(cfg);
        let (pi, cols) = (cfg.active_profile, cfg.profile().cols);
        let page = cfg.profile().active_page;
        let keys = &mut cfg.profiles[pi].pages[page].keys;
        let copied = clip_of(keys, &chosen, cols);
        if copied.is_empty() {
            return;
        }
        let count = copied.len();
        self.clip = Some(Clip { keys: copied, cut });
        if cut {
            for cell in &chosen {
                if let Some(key) = keys.get_mut(*cell) {
                    *key = DockKey::default();
                }
            }
        }
        let verb = if cut { "Cut" } else { "Copied" };
        if count > 1 {
            self.shared.set_status(format!("{verb} {count} keys: paste them anywhere, on any page, profile or pad"));
        }
    }

    /// Pastes what was cut or copied, keeping its layout, with its top left on
    /// this cell. What would fall off the pad is left out, and said so.
    pub(super) fn paste_at(&mut self, cfg: &mut Config, cell: usize) {
        let Some(clip) = &mut self.clip else { return };
        let pi = cfg.active_profile;
        let profile = &mut cfg.profiles[pi];
        let (rows, cols, kind, page) = (profile.rows, profile.cols, profile.pad, profile.active_page);
        let spots = paste_cells(&clip.keys, cell, rows, cols);
        // A cut key keeps its shortcut the first time: it's moving, not doubling.
        let keeps_shortcut = clip.cut && kind.has_shortcuts();
        let mut placed = Vec::new();
        for (index, to) in &spots {
            let source = &clip.keys[*index].2;
            if let Some(slot) = profile.pages[page].keys.get_mut(*to) {
                *slot = if keeps_shortcut { source.clone() } else { source.copied() };
                placed.push(*to);
            }
        }
        clip.cut = false;
        let left_out = clip.keys.len() - placed.len();
        self.choose(cfg, placed, Some(cell));
        if left_out > 0 {
            self.shared.set_status(format!("{left_out} of the keys didn't fit from there and weren't pasted"));
        }
    }

    /// A copy of each chosen key in the next free cells.
    pub(super) fn duplicate_selection(&mut self, cfg: &mut Config) {
        let chosen = self.selection(cfg);
        let pi = cfg.active_profile;
        let profile = &mut cfg.profiles[pi];
        let page = profile.active_page;
        let mut placed = Vec::new();
        let mut after = chosen.iter().copied().max().unwrap_or(0);
        for cell in chosen {
            let Some(key) = profile.pages[page].keys.get(cell).filter(|key| !key.is_empty()).map(DockKey::copied) else { continue };
            let Some(free) = profile.free_cell(page, after) else {
                self.shared.set_status("There's no free key on this page for more copies");
                break;
            };
            profile.pages[page].keys[free] = key;
            placed.push(free);
            after = free;
        }
        if !placed.is_empty() {
            self.choose(cfg, placed, None);
        }
    }

    /// Empties the chosen keys.
    pub(super) fn delete_selection(&mut self, cfg: &mut Config) {
        let chosen = self.selection(cfg);
        let keys = cfg.profile_mut().active_keys_mut();
        for cell in chosen {
            if let Some(key) = keys.get_mut(cell) {
                *key = DockKey::default();
            }
        }
        self.many.clear();
    }

    /// The panel under the pad while several keys are chosen.
    pub(super) fn selection_panel(&mut self, ui: &mut Ui, cfg: &mut Config) {
        let chosen = self.selection(cfg);
        heading(ui, &format!("{} KEYS CHOSEN", chosen.len()));
        ui.label(dim("Ctrl+click adds or removes a key, Shift+click picks a range, Ctrl+A picks the page, Esc goes back to one.").size(12.5));
        ui.add_space(8.0);
        let first = chosen.iter().copied().min().unwrap_or(0);
        ui.horizontal(|ui| {
            let buttons = [
                ("Cut", "Ctrl+X", KeyCommand::Cut, true),
                ("Copy", "Ctrl+C", KeyCommand::Copy, true),
                ("Paste here", "Ctrl+V", KeyCommand::Paste, self.clip.is_some()),
                ("Duplicate", "Ctrl+D", KeyCommand::Duplicate, true),
                ("Delete", "Del", KeyCommand::Delete, true),
            ];
            for (label, shortcut, command, enabled) in buttons {
                if ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(shortcut).clicked() {
                    self.key_command(cfg, first, command);
                }
            }
        });
        ui.add_space(6.0);
        ui.label(dim("A paste keeps the layout, starting at the chosen key, on any page, profile or pad.").size(12.5));
    }
}
