//! Graphs of a sensor's last hour, 6 hours or day, from Home Assistant's
//! records.
//!
//! Each sensor and stretch is fetched once, however many keys and band tiles
//! show it, and only while something shows it. Between fetches the latest
//! states are added, so the right-hand end of a graph is always now.

use super::*;
use crate::model::{BandItem, Config, HistorySpan, MeterStyle};
use std::collections::BTreeSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// How many points a graph has, the same as the band's own graphs.
pub const POINTS: usize = crate::band::HISTORY_SAMPLES;
/// How long to wait before asking again after Home Assistant refused.
const RETRY: Duration = Duration::from_secs(60);

/// One recorded state: seconds since 1970, and its number, or none for a
/// state that isn't one, such as `unavailable`.
pub(crate) type Sample = (f64, Option<f32>);

/// One sensor over one stretch: what Home Assistant gave, plus the states
/// seen since.
#[derive(Default)]
struct Series {
    samples: Vec<Sample>,
    fetched: Option<Instant>,
    tried: Option<Instant>,
}

type Key = (String, HistorySpan);

static SERIES: Mutex<BTreeMap<Key, Series>> = Mutex::new(BTreeMap::new());

fn seconds_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |since| since.as_secs_f64())
}

/// Days since 1970 for a date, and back: Howard Hinnant's well-known method,
/// which saves a whole date library for two conversions.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146097 + of_cycle - 719468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719468;
    let era = days.div_euclid(146097);
    let of_era = days - era * 146097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36524 - of_era / 146096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// A moment the way Home Assistant takes it: ISO 8601, in UTC.
pub fn iso_time(moment: SystemTime) -> String {
    let seconds = moment.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs()) as i64;
    let (year, month, day) = civil_from_days(seconds.div_euclid(86400));
    let of_day = seconds.rem_euclid(86400);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+00:00", of_day / 3600, of_day / 60 % 60, of_day % 60)
}

/// Seconds since 1970 from an ISO 8601 time, such as `2026-11-03T09:15:00.5+01:00`.
pub fn parse_time(text: &str) -> Option<f64> {
    let number = |from: usize, to: usize| text.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    let rest = text.get(19..)?;
    let zone_at = rest.find(['+', '-', 'Z']).unwrap_or(rest.len());
    let fraction: f64 = rest[..zone_at].strip_prefix('.').map_or(Some(0.0), |digits| format!("0.{digits}").parse().ok())?;
    let zone = &rest[zone_at..];
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let hours: i64 = zone.get(1..3)?.parse().ok()?;
            let minutes: i64 = zone.get(4..6).or(zone.get(3..5)).and_then(|m| m.parse().ok()).unwrap_or(0);
            (hours * 3600 + minutes * 60) * if sign == '-' { -1 } else { 1 }
        }
        _ => 0,
    };
    let whole = days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second - offset;
    Some(whole as f64 + fraction)
}

/// A time in Home Assistant's answers: seconds or milliseconds as a number,
/// depending on the answer, or ISO text from older versions.
fn time_of(value: &Value, per_second: f64) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64().map(|n| n / per_second),
        Value::String(text) => parse_time(text),
        _ => None,
    }
}

fn number_of(state: &str) -> Option<f32> {
    state.trim().parse::<f32>().ok().filter(|value| value.is_finite())
}

/// Spreads samples over `POINTS` equal steps ending at `end`. Each point is
/// the average of the numbers in its step; a step with none carries on the
/// last state before it, and a state that isn't a number leaves a gap until
/// the next number. Steps before the first sample are empty.
pub fn resample(samples: &[Sample], end: f64, length: f64) -> Vec<Option<f32>> {
    let step = length / POINTS as f64;
    let start = end - length;
    let mut sorted: Vec<Sample> = samples.iter().copied().filter(|(at, _)| *at < end).collect();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut current = None;
    let mut next = 0;
    (0..POINTS)
        .map(|point| {
            let until = start + step * (point + 1) as f64;
            let (mut sum, mut count) = (0.0f64, 0);
            let mut inside = false;
            while next < sorted.len() && sorted[next].0 < until {
                let (at, value) = sorted[next];
                if at >= until - step {
                    inside = true;
                    if let Some(value) = value {
                        sum += value as f64;
                        count += 1;
                    }
                }
                current = value;
                next += 1;
            }
            match (inside, count) {
                (true, 0) => None,
                (true, _) => Some((sum / count as f64) as f32),
                (false, _) => current,
            }
        })
        .collect()
}

/// The end of a graph's last step: the next whole step after now, so the
/// points only move along once a step has passed, not with every frame.
fn graph_end(now: f64, span: HistorySpan) -> f64 {
    let step = span.length().as_secs_f64() / POINTS as f64;
    (now / step).ceil() * step
}

/// A sensor's graph over a stretch, if it has been fetched and has any
/// samples. Each point is a number, or none where nothing is known.
pub fn points(entity: &str, span: HistorySpan) -> Option<Vec<Option<f32>>> {
    let series = lock(&SERIES);
    let found = series.get(&(entity.to_string(), span)).filter(|found| !found.samples.is_empty())?;
    let now = seconds_now();
    Some(resample(&found.samples, graph_end(now, span), span.length().as_secs_f64()))
}

/// Every sensor and stretch some key or band tile shows as a graph.
pub fn wanted(cfg: &Config) -> BTreeSet<Key> {
    let keys = cfg.profiles.iter().flat_map(|profile| &profile.pages).flat_map(|page| &page.keys);
    let from_keys = keys.filter_map(|key| {
        let binding = key.ha_binding()?;
        Some((binding.entity.clone(), binding.graph?))
    });
    let from_band = cfg.band.iter().zip(&cfg.band_meters).filter_map(|(item, meter)| match item {
        BandItem::HomeAssistant { entity, .. } if meter.on_sensor && meter.style == MeterStyle::Graph => Some((entity.clone(), meter.period?)),
        _ => None,
    });
    from_keys.chain(from_band).filter(|(entity, _)| !entity.trim().is_empty()).collect()
}

pub(crate) fn history_message(entity: &str, start: SystemTime) -> Value {
    json!({
        "type": "history/history_during_period",
        "start_time": iso_time(start),
        "entity_ids": [entity],
        "minimal_response": true,
        "no_attributes": true,
        "significant_changes_only": false,
    })
}

pub(crate) fn statistics_message(entity: &str, start: SystemTime) -> Value {
    json!({
        "type": "recorder/statistics_during_period",
        "start_time": iso_time(start),
        "statistic_ids": [entity],
        "period": "5minute",
        "types": ["mean"],
    })
}

/// The states in a `history/history_during_period` answer. Each is `s` for
/// the state and `lu` or `lc` in seconds, or the long names from older versions.
pub(crate) fn parse_history(result: &Value, entity: &str) -> Vec<Sample> {
    let Some(states) = result.get(entity).and_then(Value::as_array) else { return Vec::new() };
    states
        .iter()
        .filter_map(|state| {
            let text = state.get("s").or(state.get("state"))?.as_str()?;
            let when = ["lu", "lc", "last_updated", "last_changed"].iter().find_map(|key| time_of(state.get(key)?, 1.0))?;
            Some((when, number_of(text)))
        })
        .collect()
}

/// The averages in a `recorder/statistics_during_period` answer. Each
/// starts at `start`, in milliseconds, or ISO text from older versions.
pub(crate) fn parse_statistics(result: &Value, entity: &str) -> Vec<Sample> {
    let Some(rows) = result.get(entity).and_then(Value::as_array) else { return Vec::new() };
    rows.iter()
        .filter_map(|row| {
            let when = time_of(row.get("start")?, 1000.0)?;
            let mean = row.get("mean").and_then(Value::as_f64).map(|mean| mean as f32).filter(|mean| mean.is_finite());
            Some((when, mean))
        })
        .collect()
}

async fn answer(hub: &Hub, message: Value) -> Result<Value, HaError> {
    hub.request(message).await.unwrap_or(Err(HaError::Dropped))
}

/// Fetches a sensor's records over a stretch: the five-minute averages for
/// the longer stretches, or every change if there are none, as for a sensor
/// Home Assistant keeps no averages of.
pub(crate) async fn fetch(hub: &Hub, entity: &str, span: HistorySpan) -> Result<Vec<Sample>, HaError> {
    let start = SystemTime::now() - span.length();
    if span.uses_statistics() {
        let averages = parse_statistics(&answer(hub, statistics_message(entity, start)).await?, entity);
        if !averages.is_empty() {
            return Ok(averages);
        }
    }
    Ok(parse_history(&answer(hub, history_message(entity, start)).await?, entity))
}

/// Whether a series should be fetched now.
fn due(series: Option<&Series>, span: HistorySpan) -> bool {
    let Some(series) = series else { return true };
    match (series.fetched, series.tried) {
        (Some(fetched), _) => fetched.elapsed() >= span.refresh(),
        (None, Some(tried)) => tried.elapsed() >= RETRY,
        (None, None) => true,
    }
}

/// Adds a state seen now, unless it's the same as the last one, and lets go
/// of what has fallen off the start of the graph, keeping the last state
/// before it, which the graph's start carries on.
fn add_state(series: &mut Series, span: HistorySpan, at: f64, value: Option<f32>) {
    if series.samples.last().is_some_and(|last| last.1 == value) {
        return;
    }
    series.samples.push((at, value));
    let start = at - span.length().as_secs_f64();
    let before = series.samples.iter().filter(|(when, _)| *when < start).count();
    series.samples.drain(..before.saturating_sub(1));
}

/// Lets go of graphs nothing shows any more, and adds the latest states to
/// the rest.
fn follow(wanted: &BTreeSet<Key>, entities: &Entities) {
    let mut series = lock(&SERIES);
    series.retain(|key, _| wanted.contains(key));
    let now = seconds_now();
    for ((entity, span), found) in series.iter_mut() {
        if found.fetched.is_some() {
            if let Some(state) = entities.get(entity) {
                add_state(found, *span, now, number_of(&state.state));
            }
        }
    }
}

fn store(key: &Key, result: Result<Vec<Sample>, HaError>, entities: &Entities) {
    let mut series = lock(&SERIES);
    let found = series.entry(key.clone()).or_default();
    match result {
        Ok(samples) => {
            found.samples = samples;
            found.fetched = Some(Instant::now());
            if let Some(state) = entities.get(&key.0) {
                add_state(found, key.1, seconds_now(), number_of(&state.state));
            }
        }
        // Tried again as soon as there's a connection.
        Err(HaError::NotConnected) => found.tried = None,
        Err(error) => {
            found.tried = Some(Instant::now());
            #[cfg(not(test))]
            note(&format!("Home Assistant gave no history for {}: {error}", key.0));
            #[cfg(test)]
            let _ = error;
        }
    }
}

/// Fetches whatever is due, one at a time.
async fn refresh(shared: &Shared, wanted: &BTreeSet<Key>) -> bool {
    let mut fetched = false;
    for key in wanted {
        if !due(lock(&SERIES).get(key), key.1) {
            continue;
        }
        lock(&SERIES).entry(key.clone()).or_default().tried = Some(Instant::now());
        let result = fetch(&shared.ha, &key.0, key.1).await;
        fetched |= result.is_ok();
        store(key, result, &lock(&shared.ha.entities));
    }
    fetched
}

async fn keep_fresh(shared: Arc<Shared>) {
    let mut changes = shared.changes.subscribe();
    loop {
        let wanted = wanted(&lock(&shared.cfg));
        follow(&wanted, &lock(&shared.ha.entities));
        if refresh(&shared, &wanted).await {
            crate::pad::wake();
            shared.touch_soon();
            changes.borrow_and_update();
        }
        // Changes come in bursts, and graphs move slowly.
        tokio::select! {
            _ = changes.changed() => tokio::time::sleep(Duration::from_secs(2)).await,
            _ = tokio::time::sleep(Duration::from_secs(20)) => {}
        }
    }
}

/// Keeps the graphs in the settings fresh, for as long as the app runs.
pub fn start(shared: Arc<Shared>) {
    // After a reconnection, whatever happened while away is fetched again.
    shared.ha.on_login(Box::new(|_| {
        for series in lock(&SERIES).values_mut() {
            series.fetched = None;
            series.tried = None;
        }
    }));
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        if let Ok(runtime) = runtime {
            runtime.block_on(keep_fresh(shared));
        }
    });
}

/// For check pictures: a made-up past for every graph in the settings,
/// around each sensor's current state.
#[cfg(any(test, feature = "shots"))]
pub fn show_made_up(shared: &Shared) {
    let wanted = wanted(&lock(&shared.cfg));
    let entities = lock(&shared.ha.entities);
    let now = seconds_now();
    let mut series = lock(&SERIES);
    for (index, key) in wanted.into_iter().enumerate() {
        let Some(base) = entities.get(&key.0).and_then(|entity| number_of(&entity.state)) else { continue };
        let length = key.1.length().as_secs_f64();
        let samples = (0..=200)
            .map(|step| {
                let along = step as f64 / 200.0;
                let wave = (along * 9.0 + index as f64).sin() * 0.6 + (along * 23.0).sin() * 0.25;
                (now - length * (1.0 - along), Some(base + (wave as f32 - 0.6 * (1.0 - along) as f32) * base.abs().max(1.0) * 0.12))
            })
            .collect();
        series.insert(key, Series { samples, fetched: Some(Instant::now()), tried: None });
    }
}

#[cfg(test)]
pub(crate) fn put(entity: &str, span: HistorySpan, samples: Vec<Sample>) {
    lock(&SERIES).insert((entity.to_string(), span), Series { samples, fetched: Some(Instant::now()), tried: None });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_go_out_as_utc_iso_and_come_back() {
        assert_eq!(iso_time(UNIX_EPOCH), "1970-01-01T00:00:00+00:00");
        let moment = UNIX_EPOCH + Duration::from_secs(1_793_700_000);
        assert_eq!(iso_time(moment), "2026-11-03T10:00:00+00:00");
        assert_eq!(parse_time("2026-11-03T10:00:00+00:00"), Some(1_793_700_000.0));
        assert_eq!(parse_time("2026-11-03T11:00:00+01:00"), Some(1_793_700_000.0));
        assert_eq!(parse_time("2026-11-03T10:00:00.5Z"), Some(1_793_700_000.5));
        assert_eq!(parse_time("2024-02-29T00:00:00"), Some(1_709_164_800.0));
        assert_eq!(parse_time("not a time"), None);
        for days in [-1000, 0, 59, 60, 11_016, 20_000, 50_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
    }

    #[test]
    fn stretches_refresh_once_a_point_but_at_most_once_a_minute() {
        assert_eq!(HistorySpan::Hour.refresh(), Duration::from_secs(60));
        assert_eq!(HistorySpan::SixHours.refresh(), Duration::from_millis(337_500));
        assert_eq!(HistorySpan::Day.refresh(), Duration::from_secs(1350));
        assert!(!HistorySpan::Hour.uses_statistics() && HistorySpan::Day.uses_statistics());
        // A graph's points only move along once a whole step has passed.
        assert_eq!(graph_end(3600.0, HistorySpan::Hour), 3600.0);
        assert_eq!(graph_end(3601.0, HistorySpan::Hour), 3656.25);
    }

    #[test]
    fn nothing_known_gives_empty_points() {
        assert_eq!(resample(&[], 6400.0, 6400.0), vec![None; POINTS]);
    }

    #[test]
    fn resampling_averages_each_step_and_carries_the_last_value_on() {
        // 64 steps of 100 seconds, from 0 to 6400.
        let samples = [(250.0, Some(10.0)), (520.0, Some(20.0)), (580.0, Some(30.0)), (3000.0, Some(40.0))];
        let points = resample(&samples, 6400.0, 6400.0);
        assert_eq!(points.len(), POINTS);
        assert_eq!(&points[..2], &[None, None]);
        assert_eq!(&points[2..5], &[Some(10.0); 3]);
        assert_eq!(points[5], Some(25.0));
        assert_eq!(&points[6..30], &[Some(30.0); 24]);
        assert!(points[30..].iter().all(|point| *point == Some(40.0)));
    }

    #[test]
    fn a_state_that_isnt_a_number_leaves_a_gap_until_the_next() {
        let samples = [(50.0, Some(1.0)), (1050.0, None), (2050.0, Some(3.0)), (9000.0, Some(99.0))];
        let points = resample(&samples, 6400.0, 6400.0);
        assert!(points[..10].iter().all(|point| *point == Some(1.0)));
        assert!(points[10..20].iter().all(Option::is_none));
        assert!(points[20..].iter().all(|point| *point == Some(3.0)), "a sample after the end is left out");
        // A state from before the start carries on into it.
        let points = resample(&[(-500.0, Some(7.0))], 6400.0, 6400.0);
        assert_eq!(points, vec![Some(7.0); POINTS]);
    }

    #[test]
    fn answers_in_every_form_give_samples() {
        let history = json!({"sensor.t": [
            {"s": "21.5", "lu": 1000.5},
            {"s": "unavailable", "lu": 1100.0},
            {"state": "22", "last_changed": "1970-01-01T00:20:00+00:00"},
        ]});
        assert_eq!(parse_history(&history, "sensor.t"), vec![(1000.5, Some(21.5)), (1100.0, None), (1200.0, Some(22.0))]);
        assert!(parse_history(&history, "sensor.other").is_empty());
        let statistics = json!({"sensor.t": [
            {"start": 1_000_000, "end": 1_300_000, "mean": 19.25},
            {"start": "1970-01-01T00:21:40+00:00", "mean": null},
        ]});
        assert_eq!(parse_statistics(&statistics, "sensor.t"), vec![(1000.0, Some(19.25)), (1300.0, None)]);
    }

    #[test]
    fn requests_ask_for_what_the_graph_needs() {
        let start = UNIX_EPOCH + Duration::from_secs(1_793_700_000);
        let history = history_message("sensor.t", start);
        assert_eq!(history["type"], "history/history_during_period");
        assert_eq!(history["entity_ids"], json!(["sensor.t"]));
        assert_eq!(history["start_time"], "2026-11-03T10:00:00+00:00");
        assert_eq!((history["minimal_response"].clone(), history["no_attributes"].clone()), (json!(true), json!(true)));
        assert_eq!(history["significant_changes_only"], false);
        let statistics = statistics_message("sensor.t", start);
        assert_eq!(statistics["statistic_ids"], json!(["sensor.t"]));
        assert_eq!((statistics["period"].clone(), statistics["types"].clone()), (json!("5minute"), json!(["mean"])));
        assert!(statistics.get("units").is_none());
    }

    #[test]
    fn only_graphs_in_the_settings_are_wanted() {
        use crate::model::{Action, DockKey, HaAction};
        let mut cfg = Config::sample();
        cfg.repair();
        let key = |entity: &str, graph| DockKey {
            actions: vec![Action::HomeAssistant(HaAction { entity: entity.into(), graph, ..Default::default() })],
            ..Default::default()
        };
        let keys = &mut cfg.profiles[0].pages[0].keys;
        keys[0] = key("sensor.a", Some(HistorySpan::Hour));
        keys[1] = key("sensor.b", None);
        keys[2] = key("sensor.a", Some(HistorySpan::Hour));
        keys[3] = key("", Some(HistorySpan::Day));
        cfg.band[0] = BandItem::HomeAssistant { entity: "sensor.c".into(), label: String::new() };
        cfg.band_meters[0].period = Some(HistorySpan::Day);
        cfg.band_meters[0].on_sensor = true;
        assert_eq!(wanted(&cfg), BTreeSet::from([("sensor.a".to_string(), HistorySpan::Hour)]), "a band tile only wants a graph look");
        cfg.band_meters[0].style = MeterStyle::Graph;
        assert_eq!(wanted(&cfg).len(), 2);
    }

    #[test]
    fn new_states_are_added_and_old_ones_let_go() {
        let mut series = Series::default();
        let hour = HistorySpan::Hour;
        add_state(&mut series, hour, 0.0, Some(1.0));
        add_state(&mut series, hour, 10.0, Some(1.0));
        add_state(&mut series, hour, 20.0, Some(2.0));
        assert_eq!(series.samples, [(0.0, Some(1.0)), (20.0, Some(2.0))]);
        add_state(&mut series, hour, 5000.0, Some(3.0));
        // The last state before the start stays, since the graph carries it on.
        assert_eq!(series.samples, [(20.0, Some(2.0)), (5000.0, Some(3.0))]);
    }
}
