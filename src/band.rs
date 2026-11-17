//! The pad's side band: three small screens that show information. The
//! manufacturer's public kit lists them as display-only, 80 by 80 pixels.

use crate::ha::{fill, Entities};
use crate::icons;
use crate::model::{color_of_state, lock, BandItem, Config, Meter, MeterStyle, PictureFit, Shared};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Globalization::{GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
use windows_sys::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY, PDH_MORE_DATA,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

/// How often the processor and memory are measured. Each change sends a small
/// picture, and this pad can't answer presses during a send, so it stays calm.
const SAMPLE_EVERY: Duration = Duration::from_secs(3);
/// How many readings a graph shows: one for each pixel across its 64.
pub const HISTORY_SAMPLES: usize = 64;
/// Lets the engine readings go over 100 before they are added up.
const PDH_FMT_NOCAP100: u32 = 0x8000;

#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub cpu: u8,
    pub memory: u8,
    pub gpu: u8,
}

/// Recent readings of each band tile, oldest first.
pub type History = [Vec<u8>; 3];

/// Little pictures that can stand in for the sensors' names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BandIcon {
    Cpu,
    Gpu,
    Memory,
}

/// What one tile shows at this moment.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Face {
    pub label: String,
    pub value: String,
    /// The value's own colour, for a state that has one.
    pub value_color: Option<[u8; 3]>,
    /// Stands in for the label.
    pub icon: Option<BandIcon>,
    /// An icon from the icon set, also standing in for the label.
    pub symbol: Option<String>,
    /// A reading, which gets drawn as `meter`.
    pub percent: Option<u8>,
    pub meter: MeterStyle,
    /// Where each colour starts, in percent.
    pub levels: Vec<(u8, [u8; 3])>,
    /// Earlier readings, for a graph.
    pub history: Vec<u8>,
    pub picture: Option<String>,
    pub picture_fit: PictureFit,
    pub picture_changed: Option<SystemTime>,
    /// A colour the person chose for the tile, instead of the dark one.
    pub background: Option<[u8; 3]>,
}

fn clean(buffer: &[u16], written: i32) -> String {
    let len = (written.max(1) as usize - 1).min(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// The time, the way Windows is set to show it here.
fn local_time() -> String {
    let mut buffer = [0u16; 32];
    let written = unsafe {
        GetTimeFormatEx(
            std::ptr::null(),
            TIME_NOSECONDS,
            std::ptr::null(),
            std::ptr::null(),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    clean(&buffer, written)
}

/// Today's date, in the language Windows is set to.
fn local_date() -> (String, String) {
    let pattern = |text: &str| {
        let wide = crate::win::wide(text);
        let mut buffer = [0u16; 32];
        let written = unsafe {
            GetDateFormatEx(
                std::ptr::null(),
                0,
                std::ptr::null(),
                wide.as_ptr(),
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                std::ptr::null(),
            )
        };
        clean(&buffer, written)
    };
    (pattern("dddd"), pattern("d MMM"))
}

fn ticks(time: FILETIME) -> u64 {
    (time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64
}

/// (idle, total) time the processors have spent so far.
fn cpu_times() -> Option<(u64, u64)> {
    let empty = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut idle, mut kernel, mut user) = (empty, empty, empty);
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    // Kernel time already includes idle time.
    (ok != 0).then(|| (ticks(idle), ticks(kernel) + ticks(user)))
}

fn memory_percent() -> u8 {
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            status.dwMemoryLoad as u8
        } else {
            0
        }
    }
}

/// Reads the same Windows counters Task Manager shows for graphics cards, so
/// it works with any make of card.
struct GpuMeter {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

impl GpuMeter {
    fn new() -> Option<GpuMeter> {
        let path = crate::win::wide(r"\GPU Engine(*)\Utilization Percentage");
        unsafe {
            let mut query = std::ptr::null_mut();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let mut counter = std::ptr::null_mut();
            if PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) != 0 {
                PdhCloseQuery(query);
                return None;
            }
            PdhCollectQueryData(query);
            Some(GpuMeter { query, counter })
        }
    }

    fn read(&self) -> Option<u8> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let format = PDH_FMT_DOUBLE | PDH_FMT_NOCAP100;
            let (mut bytes, mut count) = (0u32, 0u32);
            let status = PdhGetFormattedCounterArrayW(
                self.counter,
                format,
                &mut bytes,
                &mut count,
                std::ptr::null_mut(),
            );
            if status != PDH_MORE_DATA as u32 {
                return None;
            }
            let mut buffer = vec![0u64; (bytes as usize).div_ceil(8) + 1];
            let items = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, format, &mut bytes, &mut count, items) != 0 {
                return None;
            }
            let readings = std::slice::from_raw_parts(items, count as usize).iter().map(|item| {
                let mut len = 0;
                while *item.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len));
                (name, item.FmtValue.Anonymous.doubleValue)
            });
            Some(busiest_engine(readings))
        }
    }
}

impl Drop for GpuMeter {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// Each reading is one program's use of one graphics engine. Task Manager adds
/// up the programs on each engine and shows the busiest engine.
fn busiest_engine(readings: impl Iterator<Item = (String, f64)>) -> u8 {
    let mut engines: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for (name, value) in readings {
        let engine = name.find("luid_").map_or(name.as_str(), |at| &name[at..]).to_string();
        *engines.entry(engine).or_default() += value;
    }
    engines.values().fold(0.0f64, |a, b| a.max(*b)).round().clamp(0.0, 100.0) as u8
}

fn busy_percent(before: (u64, u64), after: (u64, u64)) -> u8 {
    let idle = after.0.saturating_sub(before.0);
    let total = after.1.saturating_sub(before.1);
    if total == 0 {
        return 0;
    }
    (100 - (idle.min(total) * 100 / total)) as u8
}

/// Keeps the measurements fresh for the pad and the window.
pub fn start_measuring(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let mut before = cpu_times();
        lock(&shared.stats).memory = memory_percent();
        // This counter is the costly one, so it only runs while a tile needs it.
        let mut gpu_meter = None;
        let mut shown = Vec::new();
        loop {
            std::thread::sleep(SAMPLE_EVERY);
            let now = cpu_times();
            let cpu = match (before, now) {
                (Some(a), Some(b)) => busy_percent(a, b),
                _ => 0,
            };
            before = now;
            let wanted = lock(&shared.cfg).band.contains(&BandItem::Gpu);
            if !wanted {
                gpu_meter = None;
            } else if gpu_meter.is_none() {
                gpu_meter = GpuMeter::new();
            }
            let gpu = gpu_meter.as_ref().and_then(GpuMeter::read).unwrap_or(0);
            let stats = Stats { cpu, memory: memory_percent(), gpu };
            *lock(&shared.stats) = stats;
            let cfg = lock(&shared.cfg);
            let entities = lock(&shared.ha.entities);
            remember(&mut lock(&shared.history), &mut shown, &cfg, stats, &entities);
            drop((cfg, entities));
            // Whatever follows the changes decides how often readings matter.
            shared.touch();
        }
    });
}

fn meter_of(cfg: &Config, tile: usize) -> Meter {
    cfg.band_meters.get(tile).cloned().unwrap_or_default()
}

/// A tile's reading in percent, if what it shows can be measured.
fn reading(item: &BandItem, meter: &Meter, stats: Stats, entities: &Entities) -> Option<u8> {
    match item {
        BandItem::Cpu => Some(stats.cpu.min(100)),
        BandItem::Memory => Some(stats.memory.min(100)),
        BandItem::Gpu => Some(stats.gpu.min(100)),
        BandItem::HomeAssistant { entity, .. } if meter.on_sensor => {
            let value: f32 = entities.get(entity)?.state.parse().ok()?;
            value.is_finite().then(|| meter.percent_of(value))
        }
        _ => None,
    }
}

/// Adds the latest readings to the graphs. A tile that now shows something
/// else starts again, so one graph never runs into another.
fn remember(history: &mut History, shown: &mut Vec<BandItem>, cfg: &Config, stats: Stats, entities: &Entities) {
    for (tile, item) in cfg.band.iter().enumerate().take(history.len()) {
        if shown.get(tile) != Some(item) {
            history[tile].clear();
        }
        if let Some(percent) = reading(item, &meter_of(cfg, tile), stats, entities) {
            history[tile].push(percent);
            let extra = history[tile].len().saturating_sub(HISTORY_SAMPLES);
            history[tile].drain(..extra);
        }
    }
    shown.clone_from(&cfg.band);
}

/// For pictures and tests: made-up numbers instead of real measurements, as
/// `cpu,memory,gpu`, with a made-up past for the graphs.
#[cfg(any(test, feature = "shots"))]
pub fn show_made_up(shared: &Shared, numbers: &str) {
    let mut values = numbers.split(',').filter_map(|number| number.trim().parse::<u8>().ok());
    let mut next = || values.next().unwrap_or(0);
    let stats = Stats { cpu: next(), memory: next(), gpu: next() };
    *lock(&shared.stats) = stats;
    let cfg = lock(&shared.cfg);
    let entities = lock(&shared.ha.entities);
    let mut history = lock(&shared.history);
    for (tile, item) in cfg.band.iter().enumerate().take(history.len()) {
        let Some(now) = reading(item, &meter_of(&cfg, tile), stats, &entities) else { continue };
        history[tile] = (0..HISTORY_SAMPLES)
            .map(|sample| {
                let back = (HISTORY_SAMPLES - 1 - sample) as f32;
                let wave = (back * 0.3 + tile as f32).sin() * back * 0.5;
                (now as f32 * (1.0 - back / 130.0) + wave).clamp(0.0, 100.0) as u8
            })
            .collect();
    }
}

fn page_face(cfg: &Config) -> Face {
    let profile = cfg.main_profile();
    Face {
        label: profile.name.clone(),
        value: format!("{} / {}", profile.active_page + 1, profile.pages.len()),
        ..Face::default()
    }
}

pub fn face(item: &BandItem, cfg: &Config, stats: Stats, entities: &Entities) -> Face {
    let gauge = |label: &str, icon: BandIcon, value: u8| Face {
        label: if cfg.band_icons { String::new() } else { label.into() },
        icon: cfg.band_icons.then_some(icon),
        value: format!("{value}%"),
        percent: Some(value.min(100)),
        ..Face::default()
    };
    match item {
        BandItem::Empty => Face::default(),
        BandItem::Clock => Face { value: local_time(), ..Face::default() },
        BandItem::Date => {
            let (day, date) = local_date();
            Face { label: day, value: date, ..Face::default() }
        }
        BandItem::Cpu => gauge("CPU", BandIcon::Cpu, stats.cpu),
        BandItem::Memory => gauge("Memory", BandIcon::Memory, stats.memory),
        BandItem::Gpu => gauge("GPU", BandIcon::Gpu, stats.gpu),
        BandItem::HomeAssistant { entity, label } => {
            let found = entities.get(entity);
            let name = if label.is_empty() { "{{friendly_name}}" } else { label.as_str() };
            // A name typed for this tile is kept: it was asked for.
            let symbol = (cfg.band_icons && label.is_empty()).then(|| icons::of_entity(entity, found));
            Face {
                label: if symbol.is_some() { String::new() } else { crate::ha::title(name, entity, found) },
                symbol,
                value: fill("{{state}}{{unit}}", entity, found),
                ..Face::default()
            }
        }
        BandItem::Page => page_face(cfg),
        BandItem::Text { text } => Face { value: text.clone(), ..Face::default() },
        BandItem::Picture { path, fit } => Face {
            picture: Some(path.clone()),
            picture_fit: *fit,
            picture_changed: crate::render::modified(Some(path)),
            ..Face::default()
        },
    }
}

/// All three tiles as they should look now, from the latest measurements.
pub fn faces_now(shared: &Shared, cfg: &Config, entities: &Entities) -> Vec<Face> {
    let stats = *lock(&shared.stats);
    faces(cfg, stats, entities, &lock(&shared.history))
}

/// A sensor's graph over the stretch chosen for it, from Home Assistant's
/// records, once they're fetched. The start, before anything is known, is
/// left off; a gap later on lies on the bottom.
fn recorded(item: &BandItem, meter: &Meter) -> Option<Vec<u8>> {
    let BandItem::HomeAssistant { entity, .. } = item else { return None };
    let points = crate::ha::history::points(entity, meter.period?)?;
    let known = points.iter().skip_while(|point| point.is_none());
    Some(known.map(|point| point.map_or(0, |value| meter.percent_of(value))).collect())
}

/// All three tiles, as they should look right now.
pub fn faces(cfg: &Config, stats: Stats, entities: &Entities, history: &History) -> Vec<Face> {
    cfg.band
        .iter()
        .enumerate()
        .map(|(tile, item)| {
            let mut shown = face(item, cfg, stats, entities);
            shown.background = cfg.band_backgrounds.get(tile).copied().flatten();
            let meter = meter_of(cfg, tile);
            shown.percent = reading(item, &meter, stats, entities);
            if let BandItem::HomeAssistant { entity, .. } = item {
                shown.value_color = entities.get(entity).and_then(|found| color_of_state(&meter.states, &found.state));
            }
            if shown.percent.is_some() {
                shown.meter = meter.style;
                shown.levels = meter.percent_levels();
                // Only a graph uses these, and they change with every reading.
                if meter.style == MeterStyle::Graph {
                    shown.history = recorded(item, &meter).unwrap_or_else(|| history.get(tile).cloned().unwrap_or_default());
                }
            }
            shown
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_share_comes_from_the_difference_in_idle_time() {
        assert_eq!(busy_percent((0, 0), (25, 100)), 75);
        assert_eq!(busy_percent((100, 400), (200, 500)), 0);
        assert_eq!(busy_percent((5, 5), (5, 5)), 0);
    }

    #[test]
    fn windows_gives_a_time_and_a_date() {
        assert!(local_time().chars().any(|c| c.is_ascii_digit()));
        let (day, date) = local_date();
        assert!(!day.is_empty() && date.chars().any(|c| c.is_ascii_digit()));
    }

    #[test]
    fn memory_is_a_real_percentage() {
        let memory = memory_percent();
        assert!(memory > 0 && memory <= 100);
    }

    #[test]
    fn gpu_use_is_the_busiest_engine_with_programs_added_up() {
        let reading = |name: &str, value: f64| (name.to_string(), value);
        let readings = vec![
            reading("pid_10_luid_0x0_0xF4D9_phys_0_eng_0_engtype_3D", 30.0),
            reading("pid_20_luid_0x0_0xF4D9_phys_0_eng_0_engtype_3D", 25.0),
            reading("pid_10_luid_0x0_0xF4D9_phys_0_eng_3_engtype_VideoDecode", 40.0),
        ];
        assert_eq!(busiest_engine(readings.into_iter()), 55);
        assert_eq!(busiest_engine(vec![reading("pid_1_luid_a_eng_0", 180.0)].into_iter()), 100);
        assert_eq!(busiest_engine(std::iter::empty()), 0);
    }

    #[test]
    fn gpu_counters_give_a_reading() {
        let meter = GpuMeter::new().expect("Windows should offer the counters");
        std::thread::sleep(Duration::from_millis(1000));
        let reading = meter.read();
        println!("graphics card use: {reading:?}");
        assert!(reading.is_some());
    }

    #[test]
    fn sensors_can_swap_their_names_for_icons() {
        let mut cfg = Config::sample();
        let plain = face(&BandItem::Cpu, &cfg, Stats::default(), &Entities::new());
        assert_eq!((plain.label.as_str(), plain.icon), ("CPU", None));
        cfg.band_icons = true;
        let icon = face(&BandItem::Gpu, &cfg, Stats::default(), &Entities::new());
        assert_eq!((icon.label.as_str(), icon.icon), ("", Some(BandIcon::Gpu)));
    }

    #[test]
    fn a_band_tile_can_show_a_home_assistant_sensor() {
        let cfg = Config::sample();
        let entity = crate::ha::Entity {
            state: "28.6".into(),
            unit: "°C".into(),
            name: "Bedroom".into(),
            ..Default::default()
        };
        let entities: Entities = [("sensor.bedroom".to_string(), entity)].into_iter().collect();
        let item = BandItem::HomeAssistant { entity: "sensor.bedroom".into(), label: String::new() };
        let shown = face(&item, &cfg, Stats::default(), &entities);
        assert_eq!((shown.label.as_str(), shown.value.as_str()), ("Bedroom", "28.6°C"));
    }

    fn sensor_tile() -> (Config, Entities) {
        let mut cfg = Config::default();
        cfg.band[0] = BandItem::HomeAssistant { entity: "sensor.made_up".into(), label: String::new() };
        let entity = crate::ha::Entity { state: "25".into(), ..Default::default() };
        (cfg, [("sensor.made_up".to_string(), entity)].into_iter().collect())
    }

    #[test]
    fn a_tile_uses_its_own_meter() {
        let mut cfg = Config::default();
        cfg.band = vec![BandItem::Cpu, BandItem::Memory, BandItem::Clock];
        cfg.band_meters[1].style = MeterStyle::Gauge;
        cfg.band_meters[1].use_preset(1);
        let stats = Stats { cpu: 42, memory: 70, gpu: 0 };
        let shown = faces(&cfg, stats, &Entities::new(), &History::default());
        assert_eq!((shown[0].percent, shown[0].meter), (Some(42), MeterStyle::Bar));
        assert_eq!((shown[1].percent, shown[1].meter), (Some(70), MeterStyle::Gauge));
        assert_eq!(shown[1].levels.iter().map(|level| level.0).collect::<Vec<_>>(), [0, 60, 85]);
        assert_eq!(shown[2].percent, None);
    }

    #[test]
    fn a_sensor_only_shows_a_meter_when_asked_to() {
        let (mut cfg, entities) = sensor_tile();
        assert_eq!(faces(&cfg, Stats::default(), &entities, &History::default())[0].percent, None);
        cfg.band_meters[0] = Meter { on_sensor: true, low: 15.0, high: 35.0, ..Meter::default() };
        let shown = faces(&cfg, Stats::default(), &entities, &History::default());
        assert_eq!(shown[0].percent, Some(50));
        assert_eq!(shown[0].value, "25");
    }

    #[test]
    fn a_sensor_without_a_number_shows_no_meter() {
        let (mut cfg, mut entities) = sensor_tile();
        cfg.band_meters[0].on_sensor = true;
        entities.get_mut("sensor.made_up").unwrap().state = "unavailable".into();
        assert_eq!(faces(&cfg, Stats::default(), &entities, &History::default())[0].percent, None);
    }

    #[test]
    fn a_sensors_text_can_follow_its_state_with_or_without_a_meter() {
        let green = [0, 200, 0];
        let (mut cfg, mut entities) = sensor_tile();
        cfg.band_meters[0].states = vec![crate::model::StateColor { state: "heating".into(), color: green, text: None }];
        let shown = |entities: &Entities| faces(&cfg, Stats::default(), entities, &History::default())[0].clone();
        assert_eq!(shown(&entities).value_color, None);
        entities.get_mut("sensor.made_up").unwrap().state = "Heating".into();
        assert_eq!(shown(&entities).value_color, Some(green));
        assert_eq!(shown(&entities).percent, None);
    }

    #[test]
    fn only_a_graph_carries_the_past() {
        let mut cfg = Config::default();
        cfg.band[0] = BandItem::Cpu;
        let history: History = [vec![1, 2, 3], Vec::new(), Vec::new()];
        assert!(faces(&cfg, Stats::default(), &Entities::new(), &history)[0].history.is_empty());
        cfg.band_meters[0].style = MeterStyle::Graph;
        assert_eq!(faces(&cfg, Stats::default(), &Entities::new(), &history)[0].history, [1, 2, 3]);
    }

    #[test]
    fn graphs_keep_a_limited_past_and_restart_with_a_new_measurement() {
        let mut cfg = Config::default();
        cfg.band = vec![BandItem::Cpu, BandItem::Clock, BandItem::Clock];
        let (mut history, mut shown) = (History::default(), Vec::new());
        for cpu in 0..HISTORY_SAMPLES as u8 + 10 {
            remember(&mut history, &mut shown, &cfg, Stats { cpu, ..Stats::default() }, &Entities::new());
        }
        assert_eq!(history[0].len(), HISTORY_SAMPLES);
        assert_eq!((history[0][0], *history[0].last().unwrap()), (10, 73));
        assert!(history[1].is_empty());

        cfg.band[0] = BandItem::Memory;
        remember(&mut history, &mut shown, &cfg, Stats { memory: 5, ..Stats::default() }, &Entities::new());
        assert_eq!(history[0], [5]);
    }

    #[test]
    fn page_tile_follows_the_current_page() {
        let mut cfg = Config::sample();
        cfg.profiles[0].active_page = 1;
        assert_eq!(face(&BandItem::Page, &cfg, Stats::default(), &Entities::new()).value, "2 / 2");
    }
}
