//! Where every control of a pad is, as data: the protocol's Layout
//! (docs/integration-protocol.md, section 3.2), and the controls' names that
//! Home Assistant commands and press events use.
//!
//! Sizes are in key units: 1.0 is the width of one key, and the gaps between
//! keys as they're seen are included, so a key in column `c` starts at
//! `c * (1 + gap)`.

use super::*;

/// What sort of control it is, as the protocol names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
    Key,
    Button,
    KnobLeft,
    KnobRight,
    KnobPress,
    Band,
}

/// One control: where it is, whether it shows a picture and whether pressing
/// it does something.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Control {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ControlType,
    /// A knob's three parts share it, and their place.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub group: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub screen: bool,
    pub press: bool,
    /// The profile's cell it stands for. Band tiles have none.
    #[serde(skip)]
    pub cell: Option<usize>,
    /// Which of the 293S's band tiles it is, from the top.
    #[serde(skip)]
    pub band: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// Changes whenever anything below does.
    pub version: String,
    pub width: f32,
    pub height: f32,
    pub controls: Vec<Control>,
}

impl Layout {
    pub fn control(&self, name: &str) -> Option<&Control> {
        self.controls.iter().find(|control| control.name == name)
    }

    /// The cell a control's name stands for.
    pub fn cell_of(&self, name: &str) -> Option<usize> {
        self.control(name)?.cell
    }

    /// The name of the control on a cell.
    pub fn name_of(&self, cell: usize) -> Option<&str> {
        self.controls.iter().find(|control| control.cell == Some(cell)).map(|control| control.name.as_str())
    }
}

/// The AKP03E's knobs' first names, in the order of their cells' columns.
pub const KNOB_NAMES: [&str; 3] = ["lower_left", "top", "lower_right"];
const PARTS: [(&str, ControlType); 3] =
    [("left", ControlType::KnobLeft), ("right", ControlType::KnobRight), ("press", ControlType::KnobPress)];

/// How much of a key the 293S's band tiles are, as the window draws them.
pub const BAND_TILE: f32 = 0.82;

/// The front round the controls, in keys, wherever a pad is drawn.
pub const EDGE: f32 = 0.3;

/// The corners of a pad's front, in keys.
pub const BODY_CORNER: f32 = 0.3;

// The AKP03E's proportions, from the window's pad view (`ui::pad_view::SmallPlan`),
// in keys. Its captions are 17 pixels under 85-pixel keys.
const SPACER: f32 = 0.35;
const KNOBS_WIDE: f32 = 2.35;
const BIG_KNOB: f32 = 0.8;
const SMALL_KNOB: f32 = 0.5;
const BUTTON: f32 = 0.31;
const CAPTION: f32 = 0.2;

/// A pad's layout on the profile it's showing, with its keys as far apart
/// as they're seen.
pub fn layout_of(entry: &PadEntry, profile: &Profile) -> Layout {
    layout_for(entry.kind, entry.kind.shown_gaps(), profile)
}

/// A layout for a pad of this kind with these gaps, in percent of a key.
pub fn layout_for(kind: PadKind, gaps: [u8; 2], profile: &Profile) -> Layout {
    let gap = [gaps[0] as f32 / 100.0, gaps[1] as f32 / 100.0];
    let (controls, width, height) = match kind {
        PadKind::Main | PadKind::Dashboard => grid(kind, profile.rows, profile.cols, gap),
        PadKind::Akp03e => small(gap),
    };
    let round = |value: f32| (value * 1000.0).round() / 1000.0;
    let controls: Vec<Control> = controls
        .into_iter()
        .map(|control| Control { x: round(control.x), y: round(control.y), w: round(control.w), h: round(control.h), ..control })
        .collect();
    let (width, height) = (round(width), round(height));
    let text = serde_json::to_string(&(width, height, &controls)).unwrap_or_default();
    Layout { version: format!("{:016x}", fnv1a(text.as_bytes())), width, height, controls }
}

fn control(name: String, kind: ControlType, place: [f32; 4], cell: Option<usize>) -> Control {
    let [x, y, w, h] = place;
    let screen = matches!(kind, ControlType::Key | ControlType::Band);
    Control { name, kind, group: None, x, y, w, h, screen, press: kind != ControlType::Band, cell, band: None }
}

/// The 293S and virtual pads: rows of keys, and the 293S's band tiles to
/// the right of them.
fn grid(kind: PadKind, rows: usize, cols: usize, gap: [f32; 2]) -> (Vec<Control>, f32, f32) {
    let mut controls = Vec::new();
    for cell in 0..rows * cols {
        let (row, column) = ((cell / cols) as f32, (cell % cols) as f32);
        let place = [column * (1.0 + gap[0]), row * (1.0 + gap[1]), 1.0, 1.0];
        controls.push(control(format!("key_{}", cell + 1), ControlType::Key, place, Some(cell)));
    }
    let mut width = cols as f32 * (1.0 + gap[0]) - gap[0];
    let mut height = rows as f32 * (1.0 + gap[1]) - gap[1];
    if kind.has_band() {
        let left = cols as f32 * (1.0 + gap[0]);
        // The band is centred on the keys, as on the pad.
        let top = ((height - band_height(gap[1])) / 2.0).max(0.0);
        for tile in 0..3 {
            let place = [left, top + tile as f32 * (1.0 + gap[1]), BAND_TILE, BAND_TILE];
            let band = control(format!("band_{}", tile + 1), ControlType::Band, place, None);
            controls.push(Control { band: Some(tile), ..band });
        }
        width = left + BAND_TILE;
        height = height.max(band_height(gap[1]));
    }
    (controls, width, height)
}

/// From the top of the band's first tile to the bottom of its last, in keys.
pub fn band_height(gap: f32) -> f32 {
    2.0 * (1.0 + gap) + BAND_TILE
}

/// The AKP03E: six keys with three round buttons under them, one big knob at
/// the top right and two small ones below it.
fn small(gap: [f32; 2]) -> (Vec<Control>, f32, f32) {
    let mut controls = Vec::new();
    for key in 0..SMALL_SCREEN_KEYS {
        let (row, column) = ((key / 3) as f32, (key % 3) as f32);
        let place = [column * (1.0 + gap[0]), row * (1.0 + gap[1]), 1.0, 1.0];
        controls.push(control(format!("key_{}", key + 1), ControlType::Key, place, Some(small_cell_of_key(key))));
    }
    let button_y = 2.0 + gap[1] + 0.26;
    for button in 0..3 {
        let x = 0.5 + button as f32 * (1.0 + gap[0]) - BUTTON;
        let place = [x, button_y, 2.0 * BUTTON, 2.0 * BUTTON];
        controls.push(control(format!("button_{}", button + 1), ControlType::Button, place, Some(12 + button)));
    }
    let right = 3.0 + 2.0 * gap[0] + SPACER;
    let lower_y = 2.0 * BIG_KNOB + CAPTION + 0.1 + SMALL_KNOB;
    let knobs = [
        (right + SMALL_KNOB, lower_y, SMALL_KNOB),
        (right + KNOBS_WIDE / 2.0, BIG_KNOB, BIG_KNOB),
        (right + KNOBS_WIDE - SMALL_KNOB, lower_y, SMALL_KNOB),
    ];
    for (knob, (x, y, radius)) in knobs.into_iter().enumerate() {
        let group = format!("{}_knob", KNOB_NAMES[knob]);
        for ((part, kind), cell) in PARTS.into_iter().zip(small_knob_cells(knob)) {
            let place = [x - radius, y - radius, 2.0 * radius, 2.0 * radius];
            let part = control(format!("{group}_{part}"), kind, place, Some(cell));
            controls.push(Control { group: Some(group.clone()), ..part });
        }
    }
    let width = right + KNOBS_WIDE;
    let keys = 2.0 + gap[1] + 0.26 + 2.0 * BUTTON + CAPTION;
    let knobs = 2.0 * (BIG_KNOB + SMALL_KNOB) + 0.1 + 2.0 * CAPTION;
    (controls, width, keys.max(knobs))
}

/// The name of the control on a cell, in commands and press events. A cell
/// with no control of its own is called by its place, `key_N`.
pub fn control_name(kind: PadKind, profile: &Profile, cell: usize) -> String {
    layout_for(kind, kind.default_gaps(), profile).name_of(cell).map_or_else(|| format!("key_{}", cell + 1), str::to_string)
}

/// The cell a control's name stands for on a pad of this kind.
pub fn cell_of_name(kind: PadKind, profile: &Profile, name: &str) -> Option<usize> {
    layout_for(kind, kind.default_gaps(), profile).cell_of(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_profile() -> Profile {
        Profile::for_small_pad("Small".into())
    }

    #[test]
    fn the_names_are_the_ones_home_assistant_already_uses() {
        let small = small_profile();
        let name = |cell| control_name(PadKind::Akp03e, &small, cell);
        let names: Vec<String> = (0..18).map(name).collect();
        assert_eq!(
            names,
            [
                "key_1", "key_2", "key_3", "lower_left_knob_left", "top_knob_left", "lower_right_knob_left",
                "key_4", "key_5", "key_6", "lower_left_knob_right", "top_knob_right", "lower_right_knob_right",
                "button_1", "button_2", "button_3", "lower_left_knob_press", "top_knob_press", "lower_right_knob_press",
            ]
        );
        let main = Profile::default();
        assert_eq!(control_name(PadKind::Main, &main, 14), "key_15");
        let layout = layout_for(PadKind::Main, [52, 56], &main);
        let bands: Vec<&str> = layout.controls.iter().filter(|c| c.kind == ControlType::Band).map(|c| c.name.as_str()).collect();
        assert_eq!(bands, ["band_1", "band_2", "band_3"]);
        let dashboard = Profile::for_dashboard("Wall".into(), 2, 4);
        let layout = layout_for(PadKind::Dashboard, [52, 56], &dashboard);
        assert_eq!(layout.controls.len(), 8);
        assert_eq!(layout.cell_of("key_8"), Some(7));
        assert!(layout.cell_of("band_1").is_none() && layout.cell_of("key_9").is_none());
    }

    #[test]
    fn every_name_leads_back_to_its_cell() {
        for (kind, profile) in [(PadKind::Main, Profile::default()), (PadKind::Akp03e, small_profile())] {
            let layout = layout_for(kind, kind.default_gaps(), &profile);
            for control in layout.controls.iter().filter(|c| c.cell.is_some()) {
                assert_eq!(cell_of_name(kind, &profile, &control.name), control.cell, "{}", control.name);
                assert_eq!(control_name(kind, &profile, control.cell.unwrap()), control.name);
            }
            let names: std::collections::HashSet<&str> = layout.controls.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names.len(), layout.controls.len(), "names are unique");
        }
    }

    #[test]
    fn keys_sit_one_key_and_a_gap_apart() {
        let layout = layout_for(PadKind::Main, [50, 25], &Profile::default());
        let key = |name: &str| layout.control(name).unwrap().clone();
        assert_eq!((key("key_1").x, key("key_1").y), (0.0, 0.0));
        assert_eq!((key("key_2").x, key("key_6").y), (1.5, 1.25));
        // The band is to the right of the fifth column, one tile per row,
        // centred on the keys: (3.5 - 3.32) / 2 above it, as much below.
        let band = layout.control("band_2").unwrap();
        assert_eq!((band.x, band.y, band.w), (7.5, 1.34, BAND_TILE));
        let last = layout.control("band_3").unwrap();
        assert!((layout.height - (last.y + last.h) - layout.control("band_1").unwrap().y).abs() < 0.002);
        assert!(band.screen && !band.press);
        assert_eq!(layout.width, 7.5 + BAND_TILE);
        assert_eq!(layout.height, 3.5);
    }

    #[test]
    fn the_akp03e_controls_sit_where_the_window_draws_them() {
        let layout = layout_for(PadKind::Akp03e, [0, 0], &small_profile());
        let at = |name: &str| layout.control(name).unwrap().clone();
        // Buttons under the keys, one under each column.
        assert!(at("button_2").y > at("key_5").y + 1.0);
        assert!((at("button_2").x + at("button_2").w / 2.0 - (at("key_5").x + 0.5)).abs() < 0.001);
        // The big knob at the top right, the two small ones below it.
        let (top, left, right) = (at("top_knob_press"), at("lower_left_knob_press"), at("lower_right_knob_press"));
        assert!(top.x > at("key_3").x + 1.0 && top.y == 0.0 && top.w > left.w);
        assert!(left.y > top.y + top.h && left.x < right.x && left.y == right.y);
        // A knob's parts share their place and their group.
        assert_eq!(at("top_knob_left").x, top.x);
        assert_eq!(at("top_knob_right").group.as_deref(), Some("top_knob"));
        assert!(layout.controls.iter().all(|c| c.x + c.w <= layout.width + 0.001 && c.y + c.h <= layout.height + 0.001));
        assert!(!top.screen && top.press && at("key_1").screen);
    }

    #[test]
    fn the_version_follows_the_layout() {
        let small = Profile::for_dashboard("A".into(), 2, 4);
        let bigger = Profile::for_dashboard("B".into(), 3, 4);
        let version = |profile: &Profile, gaps| layout_for(PadKind::Dashboard, gaps, profile).version;
        assert_eq!(version(&small, [52, 56]), version(&small, [52, 56]));
        assert_ne!(version(&small, [52, 56]), version(&bigger, [52, 56]));
        assert_ne!(version(&small, [52, 56]), version(&small, [10, 56]));
        assert!(is_id(&version(&small, [52, 56])));
    }

    #[test]
    fn a_layout_has_the_protocols_field_names() {
        let layout = layout_for(PadKind::Akp03e, [35, 35], &small_profile());
        let json = serde_json::to_value(&layout).unwrap();
        let mut fields: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        fields.sort();
        assert_eq!(fields, ["controls", "height", "version", "width"]);
        let knob = json["controls"].as_array().unwrap().iter().find(|c| c["name"] == "top_knob_left").unwrap();
        let mut fields: Vec<&str> = knob.as_object().unwrap().keys().map(String::as_str).collect();
        fields.sort();
        assert_eq!(fields, ["group", "h", "name", "press", "screen", "type", "w", "x", "y"]);
        assert_eq!(knob["type"], "knob_left");
        assert_eq!(knob["group"], "top_knob");
        let key = json["controls"].as_array().unwrap().iter().find(|c| c["name"] == "key_1").unwrap();
        assert!(key.get("group").is_none() && key["type"] == "key" && key["screen"] == true);
        let main = serde_json::to_value(layout_for(PadKind::Main, [52, 56], &Profile::default())).unwrap();
        assert!(main["controls"].as_array().unwrap().iter().any(|c| c["type"] == "band" && c["press"] == false));
    }
}
