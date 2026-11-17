//! Which pads there are, the AKP03E's cells, and how a pad is doing.

use super::*;

/// What sort of pad a profile is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadKind {
    /// The StreamDock 293S.
    #[default]
    Main,
    /// An AJAZZ AKP03E: 6 screen keys, 3 plain buttons and 3 knobs.
    Akp03e,
    /// A pad with no hardware: a grid of keys that exists on screen, and in
    /// Home Assistant once its integration is there.
    Dashboard,
}

impl PadKind {
    /// The kinds a driver runs. The drivers handle one pad of each.
    pub const HARDWARE: [PadKind; 2] = [PadKind::Main, PadKind::Akp03e];

    /// The model's name, and a new pad's name.
    pub fn label(self) -> &'static str {
        match self {
            PadKind::Main => "Stream Dock 293S",
            PadKind::Akp03e => "AJAZZ AKP03E",
            PadKind::Dashboard => "Virtual pad",
        }
    }

    /// The protocol's name for it, in `PadState.kind`.
    pub fn protocol_name(self) -> &'static str {
        match self {
            PadKind::Main => "stream_dock_293s",
            PadKind::Akp03e => "akp03e",
            PadKind::Dashboard => "dashboard",
        }
    }

    /// Whether a physical pad and its driver are behind it.
    pub fn has_hardware(self) -> bool {
        self != PadKind::Dashboard
    }

    /// Whether it can show Mini Monitor.
    pub fn has_mini_monitor(self) -> bool {
        self.has_hardware()
    }

    /// Whether it has the 293S's side band.
    pub fn has_band(self) -> bool {
        self == PadKind::Main
    }

    /// Whether its keys can have keyboard shortcuts.
    pub fn has_shortcuts(self) -> bool {
        self == PadKind::Main
    }

    /// Whether every cell of its grid is a key with a screen, as on the 293S,
    /// rather than the AKP03E's mix of keys, buttons and knobs.
    pub fn is_grid(self) -> bool {
        self != PadKind::Akp03e
    }

    /// Whether this cell of one of its profiles is a key with a screen.
    pub fn cell_has_screen(self, cell: usize) -> bool {
        self.is_grid() || small_cell_has_screen(cell)
    }

    /// The gaps between keys a new pad starts with, across then down, in
    /// percent of a key.
    pub fn default_gaps(self) -> [u8; 2] {
        match self {
            PadKind::Main | PadKind::Dashboard => [52, 56],
            PadKind::Akp03e => [35, 35],
        }
    }

    /// The gaps between keys as they're seen, across then down, in percent of
    /// a key: what the window, pads on screen and Home Assistant's card draw.
    /// The pad's own `gaps` are something else: the screen hidden behind the front between keys,
    /// which a picture across keys skips.
    pub fn shown_gaps(self) -> [u8; 2] {
        match self {
            PadKind::Main | PadKind::Dashboard => [25, 25],
            PadKind::Akp03e => [26, 26],
        }
    }
}

/// One pad set up in the app: a physical pad, or a virtual pad. Its
/// profiles point to it by `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PadEntry {
    /// 16 lowercase hex characters, random (`new_id`).
    pub id: String,
    pub kind: PadKind,
    /// What the person calls it; the model's name until renamed.
    pub name: String,
    /// How bright its screen is, in percent.
    pub brightness: u8,
    /// The gaps between its keys, across then down, in percent of a key.
    pub gaps: [u8; 2],
    /// The colour of its front, in the window and on screen.
    pub face: [u8; 3],
    /// No front at all: the keys float on whatever is behind the pad. The
    /// colour above is kept for when it's turned off again.
    pub face_clear: bool,
    /// The profile it runs while the window shows another pad's.
    pub last_profile: usize,
}

impl Default for PadEntry {
    fn default() -> Self {
        PadEntry::new(PadKind::Main)
    }
}

impl PadEntry {
    /// A pad of this kind as it first comes, with a new id.
    pub fn new(kind: PadKind) -> PadEntry {
        PadEntry {
            id: new_id(),
            kind,
            name: kind.label().into(),
            brightness: DEFAULT_BRIGHTNESS,
            gaps: kind.default_gaps(),
            face: [0, 0, 0],
            face_clear: false,
            last_profile: 0,
        }
    }

    /// What its front is painted with: None when it's see-through.
    pub fn front(&self) -> Option<[u8; 3]> {
        (!self.face_clear).then_some(self.face)
    }
}

pub const SMALL_ROWS: usize = 3;

pub const SMALL_COLS: usize = 6;

pub const SMALL_SCREEN_KEYS: usize = 6;

/// The AKP03E's key pictures are 60 pixels, against 85 here, so its titles are
/// drawn larger to come out the same size.
pub(super) const SMALL_PICTURE: u32 = 60;

/// The cell of a screen key: they're the top left of the grid, three across.
pub fn small_cell_of_key(key: usize) -> usize {
    key / 3 * SMALL_COLS + key % 3
}

/// The AKP03E's knobs, in the order of their cells' columns.
pub const SMALL_KNOBS: [&str; 3] = ["Lower left knob", "Top knob", "Lower right knob"];

/// Which knob a cell of an AKP03E profile belongs to.
pub fn small_knob_of_cell(cell: usize) -> Option<usize> {
    (cell < SMALL_ROWS * SMALL_COLS && cell % SMALL_COLS >= 3).then(|| cell % SMALL_COLS - 3)
}

/// A knob's cells: turning left, turning right and pressing.
pub fn small_knob_cells(knob: usize) -> [usize; 3] {
    [3 + knob, 9 + knob, 15 + knob]
}

/// The knobs' short names, as their controls' titles start.
pub const SMALL_KNOB_SHORT: [&str; 3] = ["Lower left", "Top knob", "Lower right"];

/// What a whole AKP03E knob can be set to in one go. Each follows the
/// original app's rule for that kind of knob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnobUse {
    Volume,
    Microphone,
    Tracks,
    Pages,
    Brightness,
}

impl KnobUse {
    pub const ALL: [KnobUse; 5] = [KnobUse::Volume, KnobUse::Microphone, KnobUse::Tracks, KnobUse::Pages, KnobUse::Brightness];

    pub fn label(self) -> &'static str {
        match self {
            KnobUse::Volume => "Volume",
            KnobUse::Microphone => "Microphone",
            KnobUse::Tracks => "Music",
            KnobUse::Pages => "Pages",
            KnobUse::Brightness => "Brightness",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            KnobUse::Volume => "Turn for the speakers' volume, press to mute",
            KnobUse::Microphone => "Turn for the microphone's volume, 2% a click; press to mute or unmute it",
            KnobUse::Tracks => "Turn for the previous or next track, press to play or pause",
            KnobUse::Pages => "Turn for the previous or next page, press for the first page",
            KnobUse::Brightness => "Turn for this pad's brightness, 5% a click; pressing does nothing",
        }
    }

    /// Turning left, turning right and pressing: title, icon and action.
    fn parts(self) -> [(&'static str, &'static str, Option<Action>); 3] {
        let media = |key: MediaKey| Some(Action::Media { key });
        match self {
            KnobUse::Volume => [
                ("Volume down", "mdi:volume-minus", media(MediaKey::VolumeDown)),
                ("Volume up", "mdi:volume-plus", media(MediaKey::VolumeUp)),
                ("Mute", "mdi:volume-mute", media(MediaKey::Mute)),
            ],
            KnobUse::Microphone => [
                ("Microphone quieter", "mdi:microphone-minus", media(MediaKey::MicDown)),
                ("Microphone louder", "mdi:microphone-plus", media(MediaKey::MicUp)),
                ("Microphone on / off", MIC_ICON, media(MediaKey::MicMute)),
            ],
            KnobUse::Tracks => [
                ("Previous track", "mdi:skip-previous", media(MediaKey::PreviousTrack)),
                ("Next track", "mdi:skip-next", media(MediaKey::NextTrack)),
                ("Play / pause", "mdi:play-pause", media(MediaKey::PlayPause)),
            ],
            KnobUse::Pages => [
                ("Previous page", "mdi:page-previous-outline", Some(Action::PreviousPage)),
                ("Next page", "mdi:page-next-outline", Some(Action::NextPage)),
                ("First page", "mdi:numeric-1-box-outline", Some(Action::GoToPage { page: 1 })),
            ],
            KnobUse::Brightness => [
                ("Dimmer", "mdi:brightness-5", Some(Action::AdjustBrightness { change: -BRIGHTNESS_STEP })),
                ("Brighter", "mdi:brightness-7", Some(Action::AdjustBrightness { change: BRIGHTNESS_STEP })),
                ("press", "mdi:gesture-tap-button", None),
            ],
        }
    }

    /// The three controls of this knob set to this use.
    pub fn keys(self, knob: usize) -> [DockKey; 3] {
        self.parts().map(|(title, glyph, action)| DockKey {
            title: format!("{}\n{title}", SMALL_KNOB_SHORT[knob]),
            glyph: glyph.into(),
            color: [112, 78, 196],
            actions: action.into_iter().collect(),
            ..DockKey::default()
        })
    }

    /// What this knob is set to, if its turns and press are exactly one use.
    pub fn of(keys: &[DockKey], knob: usize) -> Option<KnobUse> {
        let cells = small_knob_cells(knob);
        KnobUse::ALL.into_iter().find(|choice| {
            choice.parts().iter().zip(cells).all(|((_, _, action), cell)| {
                keys.get(cell).is_some_and(|key| key.actions.first() == action.as_ref() && key.actions.len() <= 1)
            })
        })
    }
}

impl Page {
    /// Sets a whole AKP03E knob, its turns and its press, to one use.
    pub fn set_knob(&mut self, knob: usize, choice: KnobUse) {
        for (cell, key) in small_knob_cells(knob).into_iter().zip(choice.keys(knob)) {
            if let Some(slot) = self.keys.get_mut(cell) {
                *slot = key;
            }
        }
    }
}

/// A microphone mute key's icon; it shows the microphone crossed out while muted.
pub const MIC_ICON: &str = "mdi:microphone";
pub const MIC_OFF_ICON: &str = "mdi:microphone-off";

/// Puts keys in their new places, from a list of moves.
pub fn apply_moves(keys: &mut [DockKey], moves: &[(usize, usize)]) {
    let moving: Vec<(usize, DockKey)> = moves.iter().map(|(from, to)| (*to, keys[*from].clone())).collect();
    for (to, key) in moving {
        keys[to] = key;
    }
}

/// Whether a cell of an AKP03E profile is one of its screen keys.
pub fn small_cell_has_screen(cell: usize) -> bool {
    (0..SMALL_SCREEN_KEYS).any(|key| small_cell_of_key(key) == cell)
}

/// Where things stand with the physical pad.
#[derive(Clone, Debug, PartialEq)]
pub enum PadStatus {
    NotFound,
    OriginalRunning,
    Connected,
    MiniMonitor(String),
    Problem(String),
}

impl PadStatus {
    pub fn label(&self) -> String {
        match self {
            PadStatus::NotFound => "Stream Dock not found: check its USB cable".into(),
            PadStatus::OriginalRunning => "Waiting: close the StreamDock app to use the pad here".into(),
            PadStatus::Connected => "Stream Dock 293S connected".into(),
            PadStatus::MiniMonitor(name) => format!("Mini Monitor is on, shown as {name} in Windows"),
            PadStatus::Problem(e) => format!("Stream Dock problem: {e}"),
        }
    }

    pub fn is_working(&self) -> bool {
        matches!(self, PadStatus::Connected | PadStatus::MiniMonitor(_))
    }
}
