//! How far back a graph of a Home Assistant sensor reaches.

use super::*;
use std::time::Duration;

/// A stretch of the past taken from Home Assistant's own records. Graphs
/// without one show the app's own readings of the last few minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HistorySpan {
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "6h")]
    SixHours,
    #[serde(rename = "24h")]
    Day,
}

impl HistorySpan {
    pub const ALL: [HistorySpan; 3] = [HistorySpan::Hour, HistorySpan::SixHours, HistorySpan::Day];

    pub fn label(self) -> &'static str {
        match self {
            HistorySpan::Hour => "1 hour",
            HistorySpan::SixHours => "6 hours",
            HistorySpan::Day => "24 hours",
        }
    }

    pub fn length(self) -> Duration {
        Duration::from_secs(match self {
            HistorySpan::Hour => 3600,
            HistorySpan::SixHours => 6 * 3600,
            HistorySpan::Day => 24 * 3600,
        })
    }

    /// Home Assistant keeps five-minute averages, which are much lighter to
    /// fetch than every change over a long stretch, but too coarse for an hour.
    pub fn uses_statistics(self) -> bool {
        self != HistorySpan::Hour
    }

    /// How often it's fetched again: once per point of the graph, but not more
    /// than once a minute.
    pub fn refresh(self) -> Duration {
        (self.length() / crate::band::HISTORY_SAMPLES as u32).max(Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_before_graphs_keep_showing_live_readings() {
        let key: HaAction = serde_json::from_str(r#"{"entity": "sensor.t", "title": "{{state}}"}"#).unwrap();
        assert_eq!(key.graph, None);
        let meter: Meter = serde_json::from_str(r#"{"style": "graph", "on_sensor": true, "low": 0, "high": 40}"#).unwrap();
        assert_eq!((meter.style, meter.period), (MeterStyle::Graph, None));
        let cfg: Config = serde_json::from_str(r#"{"band_meters": [{"style": "graph"}]}"#).unwrap();
        assert_eq!(cfg.band_meters[0].period, None);
    }

    #[test]
    fn stretches_are_saved_by_their_short_names() {
        let key = HaAction { graph: Some(HistorySpan::Hour), ..Default::default() };
        let saved = serde_json::to_value(&key).unwrap();
        assert_eq!(saved["graph"], "1h");
        let meter = Meter { period: Some(HistorySpan::Day), ..Default::default() };
        let saved = serde_json::to_string(&meter).unwrap();
        assert!(saved.contains(r#""period":"24h""#), "{saved}");
        assert_eq!(serde_json::from_str::<Meter>(&saved).unwrap().period, Some(HistorySpan::Day));
        assert_eq!(serde_json::from_str::<HistorySpan>(r#""6h""#).unwrap(), HistorySpan::SixHours);
    }
}
