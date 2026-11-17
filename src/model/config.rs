//! All the settings, and the rules that keep them consistent.

use super::*;

/// Which part of the pad Mini Monitor uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorArea {
    /// The 15 keys; the band keeps showing its sensors.
    #[default]
    KeysOnly,
    /// The keys and the band together.
    KeysAndBand,
}

/// The look of the app's own window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// Follows the light or dark setting in Windows.
    #[default]
    Windows,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub profiles: Vec<Profile>,
    /// Every pad set up here, physical or on screen only. Settings from before
    /// pads had ids have none, and `repair` makes them from the old fields.
    #[serde(default)]
    pub pads: Vec<PadEntry>,
    /// This copy of the app, for the Home Assistant integration: 16 lowercase
    /// hex characters, made at the first start.
    pub instance_id: String,
    /// This PC's `machine_hash` when `instance_id` was made. A different one
    /// means the settings were copied from another PC.
    pub instance_machine: String,
    /// This PC's name in Home Assistant. Empty means the Windows name.
    pub pc_name: String,
    /// Report to the Stream Dock HA integration when it's installed.
    pub ha_integration: bool,
    /// How many Mini Monitor pictures a second dashboard cards get.
    pub ha_monitor_fps: u8,
    /// Mini Monitor pictures for dashboard cards may go over Home
    /// Assistant's local address when the app reaches it another way.
    pub ha_monitor_local: bool,
    /// The profile in the window, and on its pad unless that pad runs another.
    pub active_profile: usize,
    /// Start with only the icon near the clock.
    pub start_hidden: bool,
    /// False until the window has been shown once, so the very first start
    /// shows where the app went instead of seeming to do nothing.
    pub first_run_done: bool,
    /// x, y, width, height of the window last time it was open.
    pub window: Option<[f32; 4]>,
    /// The three tiles of the side band, top to bottom.
    pub band: Vec<BandItem>,
    /// How each of those tiles draws a measurement.
    pub band_meters: Vec<Meter>,
    /// Each band tile's background colour, when it isn't the dark one.
    pub band_backgrounds: Vec<Option<[u8; 3]>>,
    /// Asks before a key is deleted, unless told not to ask again.
    pub ask_before_delete: bool,
    /// Asks before quitting from the window, which the original does unless
    /// told not to ask again (`ShowQueryDialog/queryExitApplication`).
    pub ask_before_quit: bool,
    /// Pads' screens go dark after this long without a press, in
    /// milliseconds, as the original offers. 0 means never.
    pub screen_off_ms: u64,
    /// Pads on screen stay above other windows, rather than behaving like any
    /// other window.
    pub on_screen_on_top: bool,
    /// Mini Monitor on a pad on screen is one picture across the gaps, with
    /// square edges, like a small monitor, rather than one piece per key.
    pub on_screen_seamless: bool,
    /// Clicks and the wheel on Mini Monitor in a pad on screen act on the
    /// display at that point, as on a real monitor, and Mini Monitor stays on.
    pub on_screen_mouse: bool,
    /// Home Assistant may send commands, and hears about presses.
    pub ha_control: bool,
    /// It may also add keys that open programs or press keys on this PC.
    pub ha_control_programs: bool,
    /// Home Assistant dashboard cards may move the real pointer and click on
    /// Mini Monitor's display.
    pub ha_mouse: bool,
    /// While Home Assistant can't be reached or is starting, its keys show
    /// "HA offline" instead of their last look, dimmed with a mark.
    pub ha_offline_label: bool,
    /// The pad shows an extra Windows display instead of its keys.
    pub mini_monitor: bool,
    /// The pad that shows Mini Monitor: always one with hardware.
    pub monitor_pad: PadKind,
    pub monitor_area: MonitorArea,
    /// Treats the gaps between keys as real space, as a video wall does, so
    /// the picture keeps its shape and only what's under a gap is hidden.
    pub monitor_gaps: bool,
    pub theme: ThemeChoice,
    pub style: KeyStyle,
    /// Band sensors show small pictures instead of the words CPU, GPU and Memory.
    pub band_icons: bool,
    pub ha_url: String,
    /// Encrypted for this Windows user; never the plain token.
    pub ha_token: String,
    /// Evens out the band and the keys when Mini Monitor uses both. Below 0
    /// darkens the band by that percentage, above 0 darkens the keys.
    pub band_balance: i8,

    // Each pad's settings from before pads had ids. They are read so old
    // settings come over into `pads`, and never written again.
    #[serde(rename = "last_main_profile", skip_serializing)]
    pub old_last_main_profile: usize,
    #[serde(rename = "last_small_profile", skip_serializing)]
    pub old_last_small_profile: usize,
    #[serde(rename = "brightness", skip_serializing)]
    pub old_brightness: u8,
    #[serde(rename = "small_brightness", skip_serializing)]
    pub old_small_brightness: u8,
    #[serde(rename = "gap_across", skip_serializing)]
    pub old_gap_across: u8,
    #[serde(rename = "gap_down", skip_serializing)]
    pub old_gap_down: u8,
    #[serde(rename = "small_gap_across", skip_serializing)]
    pub old_small_gap_across: u8,
    #[serde(rename = "small_gap_down", skip_serializing)]
    pub old_small_gap_down: u8,
    #[serde(rename = "main_face", skip_serializing)]
    pub old_main_face: [u8; 3],
    #[serde(rename = "small_face", skip_serializing)]
    pub old_small_face: [u8; 3],
    #[serde(rename = "main_face_clear", skip_serializing)]
    pub old_main_face_clear: bool,
    #[serde(rename = "small_face_clear", skip_serializing)]
    pub old_small_face_clear: bool,
}

impl Default for Config {
    fn default() -> Self {
        let main = PadEntry::new(PadKind::Main);
        let first = Profile { id: new_id(), pad_id: main.id.clone(), ..Profile::default() };
        let [main_gaps, small_gaps] = PadKind::HARDWARE.map(PadKind::default_gaps);
        Config {
            profiles: vec![first],
            pads: vec![main],
            instance_id: String::new(),
            instance_machine: String::new(),
            pc_name: String::new(),
            ha_integration: true,
            ha_monitor_fps: 5,
            ha_monitor_local: true,
            active_profile: 0,
            start_hidden: true,
            first_run_done: false,
            window: None,
            band: vec![BandItem::Clock, BandItem::Cpu, BandItem::Memory],
            band_meters: vec![Meter::default(); 3],
            band_backgrounds: vec![None; 3],
            ask_before_delete: true,
            ask_before_quit: true,
            screen_off_ms: 0,
            on_screen_on_top: false,
            on_screen_seamless: false,
            on_screen_mouse: false,
            ha_control: false,
            ha_control_programs: false,
            ha_mouse: false,
            ha_offline_label: false,
            mini_monitor: false,
            monitor_pad: PadKind::Main,
            monitor_area: MonitorArea::KeysOnly,
            monitor_gaps: true,
            theme: ThemeChoice::Windows,
            style: KeyStyle::default(),
            band_icons: false,
            ha_url: String::new(),
            ha_token: String::new(),
            band_balance: 0,
            old_last_main_profile: 0,
            old_last_small_profile: 0,
            old_brightness: DEFAULT_BRIGHTNESS,
            old_small_brightness: DEFAULT_BRIGHTNESS,
            old_gap_across: main_gaps[0],
            old_gap_down: main_gaps[1],
            old_small_gap_across: small_gaps[0],
            old_small_gap_down: small_gaps[1],
            old_main_face: [0, 0, 0],
            old_small_face: [0, 0, 0],
            old_main_face_clear: false,
            old_small_face_clear: false,
        }
    }
}

impl Config {
    pub fn repair(&mut self) {
        if self.profiles.iter().all(|p| p.pad != PadKind::Main) {
            self.profiles.push(Profile::default());
        }
        for p in &mut self.profiles {
            p.repair();
        }
        self.active_profile = self.active_profile.min(self.profiles.len() - 1);
        self.repair_pads();
        self.band.resize(3, BandItem::Empty);
        self.band_meters.resize(3, Meter::default());
        self.band_backgrounds.resize(3, None);
        self.ha_monitor_fps = self.ha_monitor_fps.clamp(1, 15);
        self.band_meters.iter_mut().for_each(Meter::repair);
        self.band_balance = self.band_balance.clamp(-60, 60);
        if !self.monitor_pad.has_mini_monitor() || self.pad_profile(self.monitor_pad).is_none() {
            self.monitor_pad = PadKind::Main;
        }
    }

    /// Keeps the pads and their profiles consistent: every profile belongs to
    /// a pad of its own kind, every pad has a profile, ids are there and
    /// unique, and there's one 293S and at most one AKP03E, as the drivers
    /// handle one of each. Settings from before pads had ids get their pads
    /// here, from the old fields.
    fn repair_pads(&mut self) {
        // Virtual pads were called dashboard pads: one still named that way
        // takes the new name, with its profiles that carried the old one.
        for i in 0..self.pads.len() {
            let entry = &self.pads[i];
            let Some(rest) = entry.name.strip_prefix("Dashboard pad") else { continue };
            if entry.kind != PadKind::Dashboard || !(rest.is_empty() || rest.trim_start().parse::<u32>().is_ok()) {
                continue;
            }
            let (old, id) = (entry.name.clone(), entry.id.clone());
            let new = format!("{}{rest}", PadKind::Dashboard.label());
            if self.pads.iter().any(|other| other.name == new) {
                continue;
            }
            self.pads[i].name = new.clone();
            for profile in self.profiles.iter_mut().filter(|p| p.pad_id == id && p.name == old) {
                profile.name = new.clone();
            }
        }
        // A second 293S or AKP03E can only come from editing the file: its
        // profiles move to the first.
        let mut kinds_seen = Vec::new();
        self.pads.retain(|entry| !entry.kind.has_hardware() || {
            let first = !kinds_seen.contains(&entry.kind);
            kinds_seen.push(entry.kind);
            first
        });
        let mut ids_seen = std::collections::HashSet::new();
        for entry in &mut self.pads {
            if !is_id(&entry.id) || !ids_seen.insert(entry.id.clone()) {
                entry.id = new_id();
                ids_seen.insert(entry.id.clone());
            }
            entry.brightness = entry.brightness.min(100);
            entry.gaps = entry.gaps.map(|gap| gap.min(100));
            if entry.name.trim().is_empty() {
                entry.name = entry.kind.label().into();
            }
        }
        for index in 0..self.profiles.len() {
            self.link_profile(index);
        }
        let mut profile_ids = std::collections::HashSet::new();
        for profile in &mut self.profiles {
            if !is_id(&profile.id) || !profile_ids.insert(profile.id.clone()) {
                profile.id = new_id();
                profile_ids.insert(profile.id.clone());
            }
        }
        // An AKP03E with no profile left isn't in use any more, but its entry
        // stays, so its id, brightness, gaps and face come back with its next
        // profile. A virtual pad always keeps one.
        let used: Vec<String> = self.profiles.iter().map(|p| p.pad_id.clone()).collect();
        let empty: Vec<(String, String)> = self
            .pads
            .iter()
            .filter(|entry| entry.kind != PadKind::Akp03e && !used.contains(&entry.id))
            .map(|entry| (entry.id.clone(), entry.name.clone()))
            .collect();
        for (id, name) in empty {
            let profile = Profile::for_kind(PadKind::Dashboard, self.free_name(&name), (2, 4));
            self.profiles.push(Profile { id: new_id(), pad_id: id, ..profile });
        }
        // The physical pads first, then virtual pads in the order they were added.
        self.pads.sort_by_key(|entry| PadKind::HARDWARE.iter().position(|kind| *kind == entry.kind).unwrap_or(PadKind::HARDWARE.len()));
        for index in 0..self.pads.len() {
            let id = self.pads[index].id.clone();
            let last = self.pads[index].last_profile;
            if self.profiles.get(last).is_none_or(|p| p.pad_id != id) {
                self.pads[index].last_profile = self.profiles.iter().position(|p| p.pad_id == id).unwrap_or(0);
            }
        }
    }

    /// Points a profile at a pad of its own kind: the one it names if that
    /// fits, otherwise the first of its kind. A pad that isn't there is made,
    /// keeping the id the profile names when it can, so a virtual pad's
    /// profiles stay together.
    fn link_profile(&mut self, index: usize) {
        let (kind, wanted) = (self.profiles[index].pad, self.profiles[index].pad_id.clone());
        if self.pad(&wanted).is_some_and(|entry| entry.kind == kind) {
            return;
        }
        let existing = match kind {
            PadKind::Dashboard => None,
            _ => self.entry_of_kind(kind).map(|entry| entry.id.clone()),
        };
        let id = existing.unwrap_or_else(|| {
            let free = is_id(&wanted) && self.pad(&wanted).is_none();
            let mut entry = self.old_pad(kind);
            if free {
                entry.id = wanted.clone();
            }
            let id = entry.id.clone();
            self.pads.push(entry);
            id
        });
        self.profiles[index].pad_id = id;
    }

    /// A new pad of this kind, from the settings kept before pads had ids.
    fn old_pad(&self, kind: PadKind) -> PadEntry {
        let mut entry = PadEntry::new(kind);
        match kind {
            PadKind::Main => {
                entry.brightness = self.old_brightness;
                entry.gaps = [self.old_gap_across, self.old_gap_down];
                (entry.face, entry.face_clear) = (self.old_main_face, self.old_main_face_clear);
                entry.last_profile = self.old_last_main_profile;
            }
            PadKind::Akp03e => {
                entry.brightness = self.old_small_brightness;
                entry.gaps = [self.old_small_gap_across, self.old_small_gap_down];
                (entry.face, entry.face_clear) = (self.old_small_face, self.old_small_face_clear);
                entry.last_profile = self.old_last_small_profile;
            }
            PadKind::Dashboard => {}
        }
        entry.brightness = entry.brightness.min(100);
        entry.gaps = entry.gaps.map(|gap| gap.min(100));
        entry
    }

    /// Makes sure this app has an `instance_id` that belongs to this PC. Run
    /// at load with `machine_hash()`. Settings copied from another PC get a
    /// new instance id and new pad ids, so the two PCs never look like one in
    /// Home Assistant. Says whether anything changed.
    pub fn adopt_machine(&mut self, machine: &str) -> bool {
        if !is_id(&self.instance_id) {
            self.instance_id = new_id();
            self.instance_machine = machine.to_string();
            true
        } else if self.instance_machine != machine {
            self.renew_instance_for(machine);
            true
        } else {
            false
        }
    }

    /// A new instance id and new ids for every pad, with their profiles
    /// following. For when the integration says the ids are taken.
    pub fn renew_instance(&mut self) {
        self.renew_instance_for(&machine_hash());
    }

    /// `renew_instance` for a given `machine` value.
    pub fn renew_instance_for(&mut self, machine: &str) {
        self.instance_id = new_id();
        self.instance_machine = machine.to_string();
        let ids: Vec<String> = self.pads.iter().map(|entry| entry.id.clone()).collect();
        for id in ids {
            self.renew_pad(&id);
        }
    }

    /// Gives a pad a new id and moves its profiles over to it. Returns the new
    /// id, or None if there's no such pad.
    pub fn renew_pad(&mut self, id: &str) -> Option<String> {
        let fresh = new_id();
        let at = self.pads.iter().position(|entry| entry.id == id)?;
        self.pads[at].id = fresh.clone();
        for profile in self.profiles.iter_mut().filter(|p| p.pad_id == id) {
            profile.pad_id = fresh.clone();
        }
        Some(fresh)
    }

    /// The name this PC goes by in Home Assistant.
    pub fn pc_name_or_default(&self) -> String {
        let chosen = self.pc_name.trim();
        if !chosen.is_empty() {
            return chosen.to_string();
        }
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows PC".into())
    }

    pub fn profile(&self) -> &Profile {
        &self.profiles[self.active_profile]
    }

    /// A pad, by id.
    pub fn pad(&self, id: &str) -> Option<&PadEntry> {
        self.pads.iter().find(|entry| entry.id == id)
    }

    pub fn pad_mut(&mut self, id: &str) -> Option<&mut PadEntry> {
        self.pads.iter_mut().find(|entry| entry.id == id)
    }

    /// The pads in use: every pad with a profile. An AKP03E whose profiles
    /// were all deleted keeps its entry, but nothing shows or lists it.
    pub fn pads_in_use(&self) -> impl Iterator<Item = &PadEntry> + '_ {
        self.pads.iter().filter(|entry| self.in_use(&entry.id))
    }

    /// Whether a pad has any profile.
    pub fn in_use(&self, pad_id: &str) -> bool {
        self.profiles.iter().any(|profile| self.owner(profile).is_some_and(|entry| entry.id == pad_id))
    }

    /// The first pad of a kind in use: for the 293S and the AKP03E, the only one.
    pub fn pad_of_kind(&self, kind: PadKind) -> Option<&PadEntry> {
        self.pads_in_use().find(|entry| entry.kind == kind)
    }

    /// The entry of a kind, in use or not: an AKP03E's settings stay while
    /// it has no profiles.
    fn entry_of_kind(&self, kind: PadKind) -> Option<&PadEntry> {
        self.pads.iter().find(|entry| entry.kind == kind)
    }

    #[cfg(test)]
    pub(super) fn entry_of_kind_mut(&mut self, kind: PadKind) -> Option<&mut PadEntry> {
        self.pads.iter_mut().find(|entry| entry.kind == kind)
    }

    /// The pad a profile belongs to: the one it names, or until `repair`
    /// has linked it, the 293S or AKP03E it's for.
    pub fn pad_of_profile(&self, index: usize) -> Option<&PadEntry> {
        self.owner(self.profiles.get(index)?)
    }

    fn owner(&self, profile: &Profile) -> Option<&PadEntry> {
        match self.pad(&profile.pad_id) {
            Some(entry) if entry.kind == profile.pad => Some(entry),
            _ if profile.pad.has_hardware() => self.entry_of_kind(profile.pad),
            _ => None,
        }
    }

    fn belongs(&self, index: usize, pad_id: &str) -> bool {
        self.pad_of_profile(index).is_some_and(|entry| entry.id == pad_id)
    }

    /// The pad of the profile in the window.
    pub fn shown_pad(&self) -> Option<&PadEntry> {
        self.pad_of_profile(self.active_profile)
    }

    pub fn shown_pad_mut(&mut self) -> Option<&mut PadEntry> {
        let id = self.shown_pad_id();
        self.pad_mut(&id)
    }

    /// The id of the pad in the window.
    pub fn shown_pad_id(&self) -> String {
        self.shown_pad().map_or_else(|| self.profile().pad_id.clone(), |entry| entry.id.clone())
    }

    /// A pad's profiles, by index, in their order.
    pub fn profiles_of(&self, pad_id: &str) -> Vec<usize> {
        (0..self.profiles.len()).filter(|i| self.belongs(*i, pad_id)).collect()
    }

    /// The profile a pad is running: the one in the window if it's that pad's,
    /// otherwise the last one chosen for it.
    pub fn pad_profile_of(&self, pad_id: &str) -> Option<usize> {
        let entry = self.pad(pad_id)?;
        if self.belongs(self.active_profile, pad_id) {
            Some(self.active_profile)
        } else if self.belongs(entry.last_profile, pad_id) {
            Some(entry.last_profile)
        } else {
            self.profiles_of(pad_id).first().copied()
        }
    }

    /// The profile the first pad of a kind is running, for the drivers.
    pub fn pad_profile(&self, kind: PadKind) -> Option<usize> {
        self.pad_profile_of(&self.pad_of_kind(kind)?.id)
    }

    pub fn main_index(&self) -> usize {
        self.pad_profile(PadKind::Main).unwrap_or(0)
    }

    /// Whether this pad should be showing Mini Monitor.
    pub fn monitor_on(&self, kind: PadKind) -> bool {
        self.mini_monitor && self.monitor_pad == kind && kind.has_mini_monitor()
    }

    /// The gaps between a pad's keys, as a percentage of a key, across then down.
    pub fn gaps(&self, kind: PadKind) -> [u8; 2] {
        self.pad_of_kind(kind).map_or(kind.default_gaps(), |entry| entry.gaps)
    }

    /// The gaps between a pad's keys, as Mini Monitor allows for them.
    pub fn gaps_mut(&mut self, kind: PadKind) -> Option<&mut [u8; 2]> {
        let id = self.pad_of_kind(kind)?.id.clone();
        self.pad_mut(&id).map(|entry| &mut entry.gaps)
    }

    /// One pad's brightness, in percent.
    pub fn brightness_of(&self, kind: PadKind) -> u8 {
        self.pad_of_kind(kind).map_or(DEFAULT_BRIGHTNESS, |entry| entry.brightness)
    }

    /// Moves one pad's brightness up or down, kept to the pad's 0 to 100.
    pub fn adjust_pad_brightness(&mut self, pad_id: &str, change: i32) {
        if let Some(entry) = self.pad_mut(pad_id) {
            entry.brightness = (entry.brightness as i32 + change).clamp(0, 100) as u8;
        }
    }

    /// After a profile is renamed, keys that switch to it follow the new name.
    pub fn rename_profile_links(&mut self, old: &str, new: &str) {
        let steps = self.profiles.iter_mut().flat_map(|p| &mut p.pages).flat_map(|page| &mut page.keys).flat_map(|key| &mut key.actions);
        for action in steps {
            if let Action::SwitchProfile { name } = action {
                if name == old {
                    *name = new.to_string();
                }
            }
        }
    }

    pub fn main_profile(&self) -> &Profile {
        &self.profiles[self.main_index()]
    }

    pub fn small_pad_index(&self) -> Option<usize> {
        self.pad_profile(PadKind::Akp03e)
    }

    /// Notes what each pad is running, before the window moves.
    pub(super) fn remember_pad_profiles(&mut self) {
        let running: Vec<Option<usize>> = self.pads.iter().map(|entry| self.pad_profile_of(&entry.id)).collect();
        for (entry, running) in self.pads.iter_mut().zip(running) {
            entry.last_profile = running.unwrap_or(0);
        }
    }

    /// Shows a profile in the window, which also puts its pad on it. The other
    /// pads stay where they are.
    pub fn choose_profile(&mut self, index: usize) {
        self.remember_pad_profiles();
        self.active_profile = index.min(self.profiles.len() - 1);
    }

    /// Shows a pad in the window, on the profile it's running.
    pub fn show_pad_id(&mut self, pad_id: &str) {
        if let Some(index) = self.pad_profile_of(pad_id) {
            self.choose_profile(index);
        }
    }

    /// Puts a pad on a profile, for a key or the tray menu. The window only
    /// follows if it's showing that pad.
    pub fn run_profile(&mut self, index: usize) {
        let Some(pad_id) = self.pad_of_profile(index).map(|entry| entry.id.clone()) else { return };
        self.remember_pad_profiles();
        if let Some(entry) = self.pad_mut(&pad_id) {
            entry.last_profile = index;
        }
        if self.belongs(self.active_profile, &pad_id) {
            self.active_profile = index;
        }
    }

    /// The profile `step` places along a pad's list, wrapping round at both ends.
    pub fn profile_along_pad(&self, pad_id: &str, step: isize) -> Option<usize> {
        let list = self.profiles_of(pad_id);
        let at = list.iter().position(|i| Some(*i) == self.pad_profile_of(pad_id))? as isize;
        Some(list[(at + step).rem_euclid(list.len() as isize) as usize])
    }

    /// A new empty profile for a pad, with a free name. A virtual pad's
    /// takes the grid of the profile it's running.
    pub fn new_profile_for(&self, pad_id: &str, wanted: &str) -> Option<Profile> {
        let kind = self.pad(pad_id)?.kind;
        let grid = self.pad_profile_of(pad_id).map_or((2, 4), |i| (self.profiles[i].rows, self.profiles[i].cols));
        let profile = Profile::for_kind(kind, self.free_name(wanted), grid);
        Some(Profile { id: new_id(), pad_id: pad_id.to_string(), ..profile })
    }

    /// Adds a profile for the pad in the window, and shows it.
    pub fn add_profile(&mut self) {
        let name = (self.profiles.len() + 1..)
            .map(|n| format!("Profile {n}"))
            .find(|name| self.profiles.iter().all(|p| &p.name != name))
            .unwrap_or_default();
        let Some(profile) = self.new_profile_for(&self.shown_pad_id(), &name) else { return };
        self.profiles.push(profile);
        self.choose_profile(self.profiles.len() - 1);
    }

    /// The id of the first pad of a kind, setting one up if there's none.
    fn pad_id_of_kind(&mut self, kind: PadKind) -> String {
        if let Some(entry) = self.entry_of_kind(kind) {
            return entry.id.clone();
        }
        let entry = self.old_pad(kind);
        let id = entry.id.clone();
        self.pads.push(entry);
        id
    }

    /// Adds a profile for the AKP03E and shows it.
    pub fn add_small_pad(&mut self, name: &str) {
        let pad_id = self.pad_id_of_kind(PadKind::Akp03e);
        let profile = Profile::for_small_pad(self.free_name(name));
        self.profiles.push(Profile { id: new_id(), pad_id, ..profile });
        self.choose_profile(self.profiles.len() - 1);
    }

    /// A pad name no other pad uses, built from `wanted`.
    pub fn free_pad_name(&self, wanted: &str) -> String {
        let wanted = wanted.trim();
        let wanted = if wanted.is_empty() { PadKind::Dashboard.label() } else { wanted };
        let taken = |name: &str| self.pads.iter().any(|entry| entry.name == name);
        if !taken(wanted) {
            return wanted.to_string();
        }
        (2..).map(|n| format!("{wanted} {n}")).find(|name| !taken(name)).unwrap_or_else(|| wanted.to_string())
    }

    /// Adds a virtual pad with one empty profile of this grid, shows it in
    /// the window and returns its id.
    pub fn add_dashboard_pad(&mut self, name: &str, rows: usize, cols: usize) -> String {
        let mut entry = PadEntry::new(PadKind::Dashboard);
        entry.name = self.free_pad_name(name);
        let profile = Profile::for_dashboard(self.free_name(&entry.name), rows, cols);
        let id = entry.id.clone();
        entry.last_profile = self.profiles.len();
        self.pads.push(entry);
        self.profiles.push(Profile { id: new_id(), pad_id: id.clone(), ..profile });
        self.choose_profile(self.profiles.len() - 1);
        id
    }

    /// Renames a pad. Names are kept unique, so pads on screen and in Home
    /// Assistant can be told apart.
    pub fn rename_pad(&mut self, pad_id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || self.pads.iter().any(|entry| entry.id != pad_id && entry.name == name) {
            return Err("That pad name is empty or already used".into());
        }
        let entry = self.pad_mut(pad_id).ok_or("That pad isn't there any more")?;
        entry.name = name.to_string();
        Ok(())
    }

    /// Sets the grid of every profile of a virtual pad. Each key keeps its row
    /// and column; keys outside the new grid are removed, as
    /// `keys_outside_grid` counts them first.
    pub fn set_dashboard_grid(&mut self, pad_id: &str, rows: usize, cols: usize) {
        if self.pad(pad_id).is_none_or(|entry| entry.kind != PadKind::Dashboard) {
            return;
        }
        let (rows, cols) = (rows.clamp(1, 8), cols.clamp(1, 10));
        for profile in self.profiles.iter_mut().filter(|p| p.pad_id == pad_id) {
            let (old_rows, old_cols) = (profile.rows, profile.cols);
            if (old_rows, old_cols) == (rows, cols) {
                continue;
            }
            for page in &mut profile.pages {
                let mut old = std::mem::take(&mut page.keys).into_iter();
                let mut keys: Vec<DockKey> = (0..rows * cols).map(|_| DockKey::default()).collect();
                for r in 0..old_rows {
                    for c in 0..old_cols {
                        let Some(key) = old.next() else { break };
                        if r < rows && c < cols {
                            keys[r * cols + c] = key;
                        }
                    }
                }
                page.keys = keys;
            }
            (profile.rows, profile.cols) = (rows, cols);
            profile.repair();
        }
    }

    /// How many set-up keys, over all of a virtual pad's profiles and pages,
    /// a new grid would leave out.
    pub fn keys_outside_grid(&self, pad_id: &str, rows: usize, cols: usize) -> usize {
        self.profiles.iter().filter(|p| p.pad_id == pad_id).map(|profile| {
            profile.pages.iter().map(|page| {
                page.keys.iter().take(profile.rows * profile.cols).enumerate()
                    .filter(|(i, key)| (i / profile.cols >= rows || i % profile.cols >= cols) && !key.is_empty())
                    .count()
            }).sum::<usize>()
        }).sum()
    }

    /// Removes a virtual pad and all its profiles. The 293S and the AKP03E
    /// stay: they follow what's plugged in.
    pub fn remove_pad(&mut self, pad_id: &str) -> bool {
        if self.pad(pad_id).is_none_or(|entry| entry.kind != PadKind::Dashboard) {
            return false;
        }
        let gone = (0..self.profiles.len()).map(|i| self.belongs(i, pad_id)).collect();
        let shown = self.remove_profiles(gone);
        self.pads.retain(|entry| entry.id != pad_id);
        self.active_profile = shown.or(self.pad_profile(PadKind::Main)).unwrap_or(0);
        true
    }

    /// Removes the profiles marked `gone`, keeping each pad on the profile it
    /// runs where that one stays. Returns where the window's profile went, if
    /// it stays; the window is left for the caller to place.
    fn remove_profiles(&mut self, gone: Vec<bool>) -> Option<usize> {
        self.remember_pad_profiles();
        let mut new_index = Vec::with_capacity(gone.len());
        let mut kept = 0;
        for gone in &gone {
            new_index.push((!gone).then_some(kept));
            kept += !gone as usize;
        }
        let mut at = 0;
        self.profiles.retain(|_| {
            at += 1;
            !gone[at - 1]
        });
        let moved = |index: usize| new_index.get(index).copied().flatten();
        let firsts: Vec<usize> = self.pads.iter().map(|entry| self.profiles_of(&entry.id).first().copied().unwrap_or(0)).collect();
        for (entry, first) in self.pads.iter_mut().zip(firsts) {
            entry.last_profile = moved(entry.last_profile).unwrap_or(first);
        }
        let shown = moved(self.active_profile);
        self.active_profile = usize::MAX;
        shown
    }

    /// Deletes the profile in the window. Each pad stays on the profile it's
    /// running, and the window stays on the same pad if it has any left.
    pub fn remove_active_profile(&mut self) {
        self.remove_profile(self.active_profile);
    }

    /// Deletes any pad's profile. Each pad stays on the profile it's running
    /// where that one stays, and the window stays where it is, or on the same
    /// pad if its own profile went.
    pub fn remove_profile(&mut self, index: usize) {
        let pad_id = self.shown_pad_id();
        let gone = (0..self.profiles.len()).map(|i| i == index).collect();
        let shown = self.remove_profiles(gone);
        let shown = shown.or_else(|| self.pad_profile_of(&pad_id)).or(self.pad_profile(PadKind::Main));
        self.active_profile = shown.unwrap_or(0);
    }

    pub fn profile_mut(&mut self) -> &mut Profile {
        &mut self.profiles[self.active_profile]
    }

    /// A name no other profile uses, built from `wanted`.
    pub fn free_name(&self, wanted: &str) -> String {
        let taken = |name: &str| self.profiles.iter().any(|p| p.name == name);
        if !taken(wanted) {
            return wanted.to_string();
        }
        (2..)
            .map(|n| format!("{wanted} {n}"))
            .find(|name| !taken(name))
            .unwrap_or_else(|| wanted.to_string())
    }


    /// The buttons someone sees the first time, so the grid isn't empty.
    pub fn sample() -> Config {
        fn key(title: &str, glyph: &str, color: [u8; 3], action: Action) -> DockKey {
            DockKey {
                title: title.into(),
                glyph: glyph.into(),
                color,
                actions: vec![action],
                ..DockKey::default()
            }
        }
        const BLUE: [u8; 3] = [46, 104, 214];
        const GREEN: [u8; 3] = [32, 138, 104];
        const VIOLET: [u8; 3] = [112, 78, 196];
        const ORANGE: [u8; 3] = [196, 106, 44];
        const SLATE: [u8; 3] = [70, 80, 100];

        let media = |title: &str, glyph: &str, k: MediaKey| {
            key(title, glyph, VIOLET, Action::Media { key: k })
        };
        let open = |title: &str, glyph: &str, target: &str| {
            key(
                title,
                glyph,
                BLUE,
                Action::Open {
                    target: target.into(),
                    args: String::new(),
                },
            )
        };
        let press = |title: &str, glyph: &str, keys: &str| {
            key(title, glyph, ORANGE, Action::Shortcut { keys: keys.into() })
        };

        let mut p = Profile {
            name: "Everyday".into(),
            ..Profile::default()
        };
        let keys = &mut p.pages[0].keys;
        keys[0] = open("Calculator", "🖩", "calc.exe");
        keys[1] = open("Notepad", "📄", "notepad.exe");
        keys[2] = key(
            "Website",
            "🌐",
            GREEN,
            Action::Website {
                url: "https://example.com".into(),
            },
        );
        keys[3] = open("Downloads", "📁", "shell:Downloads");
        keys[4] = key("Next page", "➡", SLATE, Action::NextPage);
        keys[5] = media("Play / pause", "⏵", MediaKey::PlayPause);
        keys[6] = media("Previous", "⏮", MediaKey::PreviousTrack);
        keys[7] = media("Next", "⏭", MediaKey::NextTrack);
        keys[8] = media("Volume up", "🔊", MediaKey::VolumeUp);
        keys[9] = media("Volume down", "🔉", MediaKey::VolumeDown);
        keys[10] = press("Copy", "📋", "Ctrl+C");
        keys[11] = press("Paste", "📌", "Ctrl+V");
        keys[12] = press("Snip", "✂", "Win+Shift+S");
        keys[13] = media("Mute", "🔇", MediaKey::Mute);
        p.add_page();
        p.active_page = 0;

        let mut cfg = Config {
            profiles: vec![p],
            ..Config::default()
        };
        cfg.repair();
        cfg
    }
}
