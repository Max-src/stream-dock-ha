//! What a key is and does: its actions, Home Assistant binding, look and colours.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKey {
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    NextTrack,
    PreviousTrack,
    Stop,
    /// Mutes the microphone, or unmutes it.
    MicMute,
    /// The microphone's volume up by `MIC_STEP` percent.
    MicUp,
    /// The microphone's volume down by `MIC_STEP` percent.
    MicDown,
}

/// The microphone step on a knob, as in the original app: 2 percent a click,
/// kept to 0 to 100.
pub const MIC_STEP: i32 = 2;

impl MediaKey {
    pub const ALL: [MediaKey; 10] = [
        MediaKey::VolumeUp,
        MediaKey::VolumeDown,
        MediaKey::Mute,
        MediaKey::PlayPause,
        MediaKey::NextTrack,
        MediaKey::PreviousTrack,
        MediaKey::Stop,
        MediaKey::MicMute,
        MediaKey::MicUp,
        MediaKey::MicDown,
    ];

    pub fn is_microphone(self) -> bool {
        matches!(self, MediaKey::MicMute | MediaKey::MicUp | MediaKey::MicDown)
    }

    pub fn label(self) -> &'static str {
        match self {
            MediaKey::VolumeUp => "Volume up",
            MediaKey::VolumeDown => "Volume down",
            MediaKey::Mute => "Mute",
            MediaKey::PlayPause => "Play / pause",
            MediaKey::NextTrack => "Next track",
            MediaKey::PreviousTrack => "Previous track",
            MediaKey::Stop => "Stop",
            MediaKey::MicMute => "Microphone on / off",
            MediaKey::MicUp => "Microphone louder",
            MediaKey::MicDown => "Microphone quieter",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Open {
        target: String,
        #[serde(default)]
        args: String,
    },
    Website {
        url: String,
    },
    Shortcut {
        keys: String,
    },
    TypeText {
        text: String,
        /// Presses Enter once the text is typed.
        #[serde(default)]
        press_enter: bool,
    },
    Media {
        key: MediaKey,
    },
    Wait {
        ms: u32,
    },
    NextPage,
    PreviousPage,
    GoToPage {
        page: usize,
    },
    SwitchProfile {
        name: String,
    },
    /// Moves the key's pad to its next profile, wrapping round.
    NextProfile,
    /// Moves the key's pad to its previous profile, wrapping round.
    PreviousProfile,
    /// Moves the key's pad to its first profile.
    FirstProfile,
    ToggleMiniMonitor,
    /// Changes the brightness of the key's pad by this many percent, up or down.
    AdjustBrightness {
        change: i32,
    },
    HomeAssistant(HaAction),
}

impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Action::Open { .. } => "Open a program or file",
            Action::Website { .. } => "Open a website",
            Action::Shortcut { .. } => "Press keys",
            Action::TypeText { .. } => "Type text",
            Action::Media { key } if key.is_microphone() => "Microphone",
            Action::Media { .. } => "Media key",
            Action::Wait { .. } => "Wait",
            Action::NextPage => "Next page",
            Action::PreviousPage => "Previous page",
            Action::GoToPage { .. } => "Go to page",
            Action::SwitchProfile { .. } => "Switch profile",
            Action::NextProfile => "Next profile",
            Action::PreviousProfile => "Previous profile",
            Action::FirstProfile => "First profile",
            Action::ToggleMiniMonitor => "Turn on Mini Monitor",
            Action::AdjustBrightness { .. } => "Change brightness",
            Action::HomeAssistant(_) => "Home Assistant",
        }
    }
}

/// The outline of every key, on the pad and in the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyShape {
    #[default]
    Square,
    Rounded,
    Circle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPlace {
    #[default]
    Bottom,
    Middle,
    Top,
}

/// The usual title size in points.
pub(super) const DEFAULT_TITLE_POINTS: u8 = 8;

/// How all the keys look. Set once for the whole app.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyStyle {
    pub shape: KeyShape,
    /// Corner size for rounded keys, in pixels.
    pub corner: u8,
    /// A file in the Windows fonts folder; empty means the built-in font.
    pub font: String,
    pub size: u8,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
    pub place: TextPlace,
}

impl Default for KeyStyle {
    fn default() -> Self {
        KeyStyle {
            shape: KeyShape::Square,
            corner: 14,
            font: String::new(),
            size: ((DEFAULT_TITLE_POINTS as u32 * 96 + 36) / 72) as u8,
            bold: false,
            italic: false,
            color: [255, 255, 255],
            place: TextPlace::Bottom,
        }
    }
}

/// How a picture is shown on a key or a band tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PictureFit {
    /// All of it, smaller if need be, with bars where its shape differs.
    #[default]
    Fit,
    /// Covers everything, cutting off whatever sticks out.
    Fill,
    /// Covers everything by changing the picture's shape.
    Stretch,
    /// Not resized: a large picture shows only its middle.
    Original,
}

impl PictureFit {
    pub const ALL: [PictureFit; 4] = [PictureFit::Fit, PictureFit::Fill, PictureFit::Stretch, PictureFit::Original];

    pub fn label(self) -> &'static str {
        match self {
            PictureFit::Fit => "Whole picture",
            PictureFit::Fill => "Fill, cutting off the edges",
            PictureFit::Stretch => "Stretch to fill",
            PictureFit::Original => "Original size, centred",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            PictureFit::Fit => "Shows all of it, with bars if its shape differs from the key",
            PictureFit::Fill => "Keeps its shape and covers the key, so what sticks out is cut off",
            PictureFit::Stretch => "Covers the key by changing the picture's shape",
            PictureFit::Original => "Isn't resized, so a large picture shows only its middle",
        }
    }
}

/// A colour for one exact state of an entity, such as `heating`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StateColor {
    pub state: String,
    pub color: [u8; 3],
    /// A key's text colour for this state. Without one it keeps the usual
    /// colour, which is what the original does for a state with no colour.
    #[serde(default)]
    pub text: Option<[u8; 3]>,
}

/// The row for this state, if any. Capitals don't matter, and an unfinished
/// empty row never matches.
pub(super) fn rule_of_state<'a>(rules: &'a [StateColor], state: &str) -> Option<&'a StateColor> {
    let state = state.trim();
    rules.iter().find(|rule| !rule.state.trim().is_empty() && rule.state.trim().eq_ignore_ascii_case(state))
}

pub fn color_of_state(rules: &[StateColor], state: &str) -> Option<[u8; 3]> {
    rule_of_state(rules, state).map(|rule| rule.color)
}

pub fn text_color_of_state(rules: &[StateColor], state: &str) -> Option<[u8; 3]> {
    rule_of_state(rules, state).and_then(|rule| rule.text)
}

/// A key tied to a Home Assistant entity: what pressing it does, and how it
/// shows the entity's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HaAction {
    pub entity: String,
    /// Empty means pressing does nothing, as for a sensor.
    pub service: String,
    /// Extra JSON for the service, usually empty.
    pub data: String,
    /// Text on the key, with {{friendly_name}}, {{state}}, {{unit}} or any
    /// attribute filled in.
    pub title: String,
    pub on_title: String,
    pub off_title: String,
    pub on_image: Option<String>,
    pub off_image: Option<String>,
    /// Background by state; none means the key's own colour.
    pub on_color: Option<[u8; 3]>,
    pub off_color: Option<[u8; 3]>,
    /// The drawn switch's track by state; none means its usual colours.
    pub on_track: Option<[u8; 3]>,
    pub off_track: Option<[u8; 3]>,
    /// Draws a switch on the key for on and off states.
    pub show_switch: bool,
    /// Background for exact states. These come first.
    pub state_colors: Vec<StateColor>,
    /// Background for numbers, each colour from its value up.
    pub level_colors: Vec<MeterLevel>,
    /// A graph of the sensor's recent past, drawn behind the text.
    pub graph: Option<HistorySpan>,
    /// The first press only asks "Press again"; a second one within a few
    /// seconds runs it, so a key such as Restart Home Assistant can't be
    /// pressed by accident.
    pub confirm: bool,
}

impl Default for HaAction {
    fn default() -> Self {
        HaAction {
            entity: String::new(),
            service: String::new(),
            data: String::new(),
            title: "{{friendly_name}}".into(),
            on_title: String::new(),
            off_title: String::new(),
            on_image: None,
            off_image: None,
            on_color: None,
            off_color: None,
            on_track: None,
            off_track: None,
            show_switch: true,
            state_colors: Vec::new(),
            level_colors: Vec::new(),
            graph: None,
            confirm: false,
        }
    }
}

/// The service a premade Restart Home Assistant key calls.
pub const RESTART_SERVICE: &str = "homeassistant.restart";

impl HaAction {
    /// The premade key that restarts Home Assistant: no entity, and it asks
    /// for a second press.
    pub fn restart_home_assistant() -> HaAction {
        HaAction { service: RESTART_SERVICE.into(), title: "Restart HA".into(), confirm: true, ..HaAction::default() }
    }

    /// Whether it only calls a service, with no entity to show.
    pub fn service_only(&self) -> bool {
        self.entity.trim().is_empty() && !self.service.trim().is_empty()
    }

    /// Whether it restarts Home Assistant.
    pub fn restarts_home_assistant(&self) -> bool {
        self.service.trim() == RESTART_SERVICE
    }

    /// Whether any of its settings for on and off are in use.
    pub fn uses_on_off(&self) -> bool {
        self.on_color.is_some()
            || self.off_color.is_some()
            || self.on_track.is_some()
            || self.off_track.is_some()
            || self.on_image.is_some()
            || self.off_image.is_some()
            || !self.on_title.is_empty()
            || !self.off_title.is_empty()
    }

    /// The background chosen for this state: an exact match, or else the
    /// highest number it has reached.
    pub fn color_for(&self, state: &str) -> Option<[u8; 3]> {
        color_of_state(&self.state_colors, state).or_else(|| {
            let value: f32 = state.trim().parse().ok()?;
            let reached = self.level_colors.iter().filter(|level| level.from <= value);
            reached.max_by(|a, b| a.from.total_cmp(&b.from)).map(|level| level.color)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockKey {
    pub title: String,
    pub color: [u8; 3],
    /// One symbol, or an icon written `mdi:name`.
    pub glyph: String,
    pub image: Option<String>,
    /// How this key shows its pictures, including those for a state.
    pub picture_fit: PictureFit,
    /// Text only: no symbol, picture or switch, whatever the key is tied to.
    pub no_icon: bool,
    /// A shortcut that runs this key from anywhere, even with the window closed.
    pub hotkey: String,
    pub actions: Vec<Action>,
    /// How many keys across and down a camera covers, counting this one.
    pub span: [u8; 2],
    /// Whether a stretched camera's picture allows for the gaps between keys,
    /// like Mini Monitor, or runs straight from one key into the next.
    pub camera_gaps: bool,
    /// Whether Home Assistant may press it. Only the person sets this.
    #[serde(skip_serializing_if = "RemoteAccess::is_auto")]
    pub remote: RemoteAccess,
}

/// Whether Home Assistant (its actions, events and dashboard cards) may press
/// a key. Presses on the pad, on screen and from shortcuts don't ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAccess {
    /// Yes, unless one of its steps acts on this PC.
    #[default]
    Auto,
    Allow,
    Deny,
}

impl RemoteAccess {
    pub fn is_auto(&self) -> bool {
        *self == RemoteAccess::Auto
    }
}

impl Default for DockKey {
    fn default() -> Self {
        DockKey {
            title: String::new(),
            color: [44, 49, 61],
            glyph: String::new(),
            image: None,
            picture_fit: PictureFit::Fit,
            no_icon: false,
            hotkey: String::new(),
            actions: Vec::new(),
            span: [1, 1],
            camera_gaps: false,
            remote: RemoteAccess::Auto,
        }
    }
}

impl DockKey {
    /// A copy to put elsewhere. A shortcut can only belong to one key.
    pub fn copied(&self) -> DockKey {
        DockKey { hotkey: String::new(), ..self.clone() }
    }

    /// The key's first Home Assistant step, which decides how it shows state.
    pub fn ha_binding(&self) -> Option<&HaAction> {
        self.actions.iter().find_map(|action| match action {
            Action::HomeAssistant(binding) => Some(binding),
            _ => None,
        })
    }

    /// The Home Assistant camera this key shows, if any.
    pub fn camera(&self) -> Option<&str> {
        self.ha_binding().map(|binding| binding.entity.as_str()).filter(|entity| entity.starts_with("camera."))
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
            && self.title.is_empty()
            && self.glyph.is_empty()
            && self.image.is_none()
            && self.hotkey.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub keys: Vec<DockKey>,
}
