//! The side band's tiles and how each draws its measurement.

use super::*;

/// How a measurement is drawn on a band tile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterStyle {
    #[default]
    Bar,
    Blocks,
    Gauge,
    Ring,
    Graph,
    Number,
}

impl MeterStyle {
    pub const ALL: [MeterStyle; 6] = [
        MeterStyle::Bar,
        MeterStyle::Blocks,
        MeterStyle::Gauge,
        MeterStyle::Ring,
        MeterStyle::Graph,
        MeterStyle::Number,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MeterStyle::Bar => "Bar: one colour, for the level reached",
            MeterStyle::Blocks => "Blocks: each block keeps the colour of its level",
            MeterStyle::Gauge => "Gauge: a dial with a needle, showing every level",
            MeterStyle::Ring => "Ring: a circle that fills up",
            MeterStyle::Graph => "Graph: the last three minutes or so",
            MeterStyle::Number => "Number: just the value, in the colour of its level",
        }
    }
}

/// From this value up, a meter takes this colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeterLevel {
    pub from: f32,
    pub color: [u8; 3],
}

pub const MAX_METER_LEVELS: usize = 6;

pub(super) const METER_BLUE: [u8; 3] = [79, 140, 255];

pub(super) const METER_RED: [u8; 3] = [226, 92, 92];

pub(super) const METER_GREEN: [u8; 3] = [72, 196, 120];

pub(super) const METER_ORANGE: [u8; 3] = [243, 156, 56];

pub(super) const METER_YELLOW: [u8; 3] = [240, 208, 72];

/// Ready-made levels, as shares of the meter's range.
pub const METER_PRESETS: [(&str, &[(f32, [u8; 3])]); 5] = [
    ("Blue, red when nearly full", &[(0.0, METER_BLUE), (0.9, METER_RED)]),
    ("Green, orange, red", &[(0.0, METER_GREEN), (0.6, METER_ORANGE), (0.85, METER_RED)]),
    ("Red, orange, green (low is bad, like a battery)", &[(0.0, METER_RED), (0.2, METER_ORANGE), (0.5, METER_GREEN)]),
    (
        "Blue, green, yellow, orange, red",
        &[(0.0, METER_BLUE), (0.4, METER_GREEN), (0.6, METER_YELLOW), (0.75, METER_ORANGE), (0.9, METER_RED)],
    ),
    ("One colour", &[(0.0, METER_BLUE)]),
];

/// How one band tile draws its measurement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meter {
    pub style: MeterStyle,
    /// In rising order. The first one starts at the bottom of the range.
    pub levels: Vec<MeterLevel>,
    /// Home Assistant sensors only show a meter when asked to, since not
    /// every sensor gives a number.
    pub on_sensor: bool,
    /// The range of a Home Assistant sensor. Everything else is 0 to 100.
    pub low: f32,
    pub high: f32,
    /// The colour of a Home Assistant sensor's value for exact states.
    pub states: Vec<StateColor>,
    /// How far back a Home Assistant sensor's graph reaches, from its
    /// records. None means the app's own readings of the last few minutes.
    pub period: Option<HistorySpan>,
}

impl Default for Meter {
    fn default() -> Self {
        let mut meter =
            Meter { style: MeterStyle::Bar, levels: Vec::new(), on_sensor: false, low: 0.0, high: 100.0, states: Vec::new(), period: None };
        meter.use_preset(0);
        meter
    }
}

impl Meter {
    pub(super) fn at_share(&self, share: f32) -> f32 {
        self.low + share * (self.high - self.low)
    }

    pub fn use_preset(&mut self, preset: usize) {
        self.levels = METER_PRESETS[preset].1
            .iter()
            .map(|(share, color)| MeterLevel { from: self.at_share(*share), color: *color })
            .collect();
    }

    /// The ready-made levels in use, if they haven't been changed.
    pub fn preset(&self) -> Option<usize> {
        METER_PRESETS.iter().position(|(_, levels)| {
            levels.len() == self.levels.len()
                && levels.iter().zip(&self.levels).all(|((share, color), level)| {
                    *color == level.color && (self.at_share(*share) - level.from).abs() < 0.001
                })
        })
    }

    /// A new level halfway between the top one and the end of the range.
    pub fn add_level(&mut self) {
        let Some(top) = self.levels.last().copied() else { return };
        if self.levels.len() < MAX_METER_LEVELS && top.from < self.high {
            self.levels.push(MeterLevel { from: (top.from + self.high) / 2.0, ..top });
        }
    }

    /// How far along the range a value is, in percent.
    pub fn percent_of(&self, value: f32) -> u8 {
        ((value - self.low) / (self.high - self.low) * 100.0).round().clamp(0.0, 100.0) as u8
    }

    /// The levels as percentages of the range, which is what gets drawn.
    pub fn percent_levels(&self) -> Vec<(u8, [u8; 3])> {
        self.levels.iter().map(|level| (self.percent_of(level.from), level.color)).collect()
    }

    /// Puts right what can't be drawn: no levels, an empty range, levels out
    /// of order or outside the range.
    pub fn repair(&mut self) {
        if self.levels.is_empty() {
            self.use_preset(0);
        }
        self.levels.truncate(MAX_METER_LEVELS);
        if !self.low.is_finite() || !self.high.is_finite() {
            (self.low, self.high) = (0.0, 100.0);
        }
        if self.high <= self.low {
            self.high = self.low + 1.0;
        }
        let (low, high) = (self.low, self.high);
        self.levels.sort_by(|a, b| a.from.total_cmp(&b.from));
        for level in &mut self.levels {
            level.from = level.from.clamp(low, high);
        }
        self.levels[0].from = low;
    }
}

/// What one tile of the pad's side band shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BandItem {
    Empty,
    Clock,
    Date,
    Cpu,
    Memory,
    Gpu,
    Page,
    HomeAssistant { entity: String, label: String },
    Text { text: String },
    Picture {
        path: String,
        #[serde(default)]
        fit: PictureFit,
    },
}

impl BandItem {
    pub fn choices() -> [BandItem; 10] {
        [
            BandItem::Clock,
            BandItem::Date,
            BandItem::Cpu,
            BandItem::Memory,
            BandItem::Gpu,
            BandItem::Page,
            BandItem::HomeAssistant { entity: String::new(), label: String::new() },
            BandItem::Text { text: String::new() },
            BandItem::Picture { path: String::new(), fit: PictureFit::Fit },
            BandItem::Empty,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            BandItem::Empty => "Nothing",
            BandItem::Clock => "Clock",
            BandItem::Date => "Date",
            BandItem::Cpu => "Processor use",
            BandItem::Memory => "Memory use",
            BandItem::Gpu => "Graphics card use",
            BandItem::Page => "Current profile and page",
            BandItem::HomeAssistant { .. } => "Home Assistant sensor",
            BandItem::Text { .. } => "Your own text",
            BandItem::Picture { .. } => "A picture",
        }
    }
}
