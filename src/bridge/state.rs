//! The `state` message: everything about this PC and its pads, in full
//! (docs/integration-protocol.md, section 3.2).

use crate::frames;
use crate::ha::Entities;
use crate::model::layout::{BODY_CORNER, EDGE};
use crate::model::{lock, Config, KeyShape, KeyStyle, PadEntry, PadKind, Profile, Shared};
use crate::render::{band_shape, BAND_SIZE, KEY_SIZE};
use serde_json::{json, Value};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// The whole `state` message, as it would go out now.
pub fn state_message(shared: &Shared) -> Value {
    let pc = pc_values(shared);
    let cfg = lock(&shared.cfg);
    let entities = lock(&shared.ha.entities);
    let pads: Vec<Value> = cfg.pads_in_use().filter_map(|entry| pad_state(shared, &cfg, &entities, entry)).collect();
    json!({
        "type": "stream_dock_ha/state",
        "name": cfg.pc_name_or_default(),
        "control": cfg.ha_control,
        "pc": pc,
        "pads": pads,
    })
}

/// A `state` without the PC's CPU, memory and GPU, which change all the time,
/// to tell whether anything else changed.
pub fn without_stats(state: &Value) -> Value {
    let mut state = state.clone();
    if let Some(pc) = state.get_mut("pc").and_then(Value::as_object_mut) {
        for field in ["cpu", "memory", "gpu"] {
            pc.remove(field);
        }
    }
    state
}

/// The PC's own values.
fn pc_values(shared: &Shared) -> Value {
    let stats = *lock(&shared.stats);
    let last_press = *lock(&shared.last_press);
    json!({
        "mic_muted": crate::media::microphone_muted(),
        "cpu": stats.cpu,
        "memory": stats.memory,
        "gpu": stats.gpu,
        "last_press": last_press.map(utc_text),
    })
}

/// Whether the physical pad is plugged in and working. A virtual pad has
/// nothing to plug in, so it always is.
fn connected(shared: &Shared, kind: PadKind) -> bool {
    match kind {
        PadKind::Main => lock(&shared.pad_status).is_working(),
        PadKind::Akp03e => crate::small_pad::is_connected(),
        PadKind::Dashboard => true,
    }
}

fn hex(value: u64) -> String {
    format!("{value:016x}")
}

/// How the pad's front and keys look, so the card draws it as the window does.
fn body(cfg: &Config, entry: &PadEntry, profile: &Profile) -> Value {
    // Corners in parts of a picture's width, as the pictures are cut.
    let corner = |style: &KeyStyle, size: u32| match style.shape {
        KeyShape::Square => 0.0,
        KeyShape::Rounded => (style.corner as f32 / size as f32).min(0.5),
        KeyShape::Circle => 0.5,
    };
    let round = |value: f32| (value as f64 * 1000.0).round() / 1000.0;
    json!({
        "front": entry.front(),
        "edge": round(EDGE),
        "corner": round(BODY_CORNER),
        "key_corner": round(corner(&profile.style_on_pad(&cfg.style), KEY_SIZE)),
        "band_corner": round(corner(&band_shape(&cfg.style), BAND_SIZE)),
    })
}

/// One pad's PadState, or nothing for a pad not in use yet.
fn pad_state(shared: &Shared, cfg: &Config, entities: &Entities, entry: &PadEntry) -> Option<Value> {
    let index = cfg.pad_profile_of(&entry.id)?;
    let profile = &cfg.profiles[index];
    let frame = frames::pad_frame(shared, cfg, entities, &entry.id)?;
    let screen_on = !crate::screen_off::pad_asleep(cfg, &entry.id);
    // The band follows the PC's numbers every few seconds, which would make
    // the look change all the time, so only the keys count.
    let keys: Vec<u64> = frame.cells.iter().map(|shown| shown.hash).collect();
    let body = body(cfg, entry, profile);
    let look = BuildHasherDefault::<DefaultHasher>::default().hash_one((keys, &frame.layout.version, screen_on, body.to_string()));
    let profiles: Vec<Value> = cfg
        .profiles_of(&entry.id)
        .into_iter()
        .map(|i| json!({ "id": cfg.profiles[i].id, "name": cfg.profiles[i].name, "pages": cfg.profiles[i].pages.len() }))
        .collect();
    Some(json!({
        "id": entry.id,
        "kind": entry.kind.protocol_name(),
        "model": entry.kind.label(),
        "name": entry.name,
        "connected": connected(shared, entry.kind),
        "screen_on": screen_on,
        "brightness": entry.brightness.min(100),
        "mini_monitor": entry.kind.has_mini_monitor().then(|| cfg.monitor_on(entry.kind)),
        "profile": { "id": profile.id, "name": profile.name },
        "profiles": profiles,
        "page": profile.active_page + 1,
        "pages": profile.pages.len(),
        "look": hex(look),
        "alert": crate::alerts::active(shared, &entry.id).is_some(),
        "layout": frame.layout,
        "body": body,
    }))
}

/// A time as UTC in ISO 8601, to the second, like `2026-11-03T09:12:44Z`.
pub fn utc_text(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (days, of_day) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_date(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", of_day / 3600, of_day / 60 % 60, of_day % 60)
}

/// The calendar date so many days after 1970-01-01, after Howard Hinnant's
/// `civil_from_days`.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn times_are_written_in_utc_like_the_protocol_wants() {
        let at = |seconds: u64| utc_text(UNIX_EPOCH + Duration::from_secs(seconds));
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(at(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(at(1_793_611_964), "2026-11-02T09:32:44Z");
        assert_eq!(at(4_107_542_399), "2100-02-28T23:59:59Z");
    }

    #[test]
    fn the_pcs_numbers_alone_are_told_apart_from_other_changes() {
        let a = json!({"name": "PC", "pc": {"cpu": 1, "memory": 2, "gpu": 3, "mic_muted": false}});
        let b = json!({"name": "PC", "pc": {"cpu": 9, "memory": 8, "gpu": 7, "mic_muted": false}});
        let c = json!({"name": "PC", "pc": {"cpu": 1, "memory": 2, "gpu": 3, "mic_muted": true}});
        assert_eq!(without_stats(&a), without_stats(&b));
        assert_ne!(without_stats(&a), without_stats(&c));
    }

    #[test]
    fn an_akp03e_with_no_profiles_left_isnt_listed() {
        let mut cfg = Config::sample();
        cfg.add_small_pad("Small");
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        cfg.remove_active_profile();
        cfg.repair();
        assert!(cfg.pad(&small).is_some(), "its settings are kept");
        let (shared, _opened) = Shared::new(cfg, true);
        let state = state_message(&shared);
        let ids: Vec<&str> = state["pads"].as_array().unwrap().iter().filter_map(|pad| pad["id"].as_str()).collect();
        assert_eq!(ids.len(), 1);
        assert!(!ids.contains(&small.as_str()));
    }

    #[test]
    fn the_state_has_every_pad_with_its_layout() {
        let mut cfg = Config::sample();
        cfg.pc_name = "Desk PC".into();
        cfg.ha_control = true;
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        cfg.repair();
        let (shared, _opened) = Shared::new(cfg, true);
        *lock(&shared.last_press) = Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let state = state_message(&shared);
        assert_eq!((state["type"].clone(), state["name"].clone(), state["control"].clone()), (json!("stream_dock_ha/state"), json!("Desk PC"), json!(true)));
        assert_eq!(state["pc"]["last_press"], "2023-11-14T22:13:20Z");
        let pads = state["pads"].as_array().unwrap();
        assert_eq!(pads.len(), 2);
        let main = &pads[0];
        assert_eq!((main["kind"].clone(), main["model"].clone(), main["mini_monitor"].clone()), (json!("stream_dock_293s"), json!("Stream Dock 293S"), json!(false)));
        assert_eq!(main["layout"]["controls"].as_array().unwrap().len(), 18);
        assert!(main["layout"]["controls"][0].get("cell").is_none(), "internal fields stay in");
        let dashboard = &pads[1];
        assert_eq!((dashboard["id"].clone(), dashboard["kind"].clone(), dashboard["connected"].clone()), (json!(wall), json!("dashboard"), json!(true)));
        assert_eq!((dashboard["mini_monitor"].clone(), dashboard["screen_on"].clone(), dashboard["alert"].clone()), (Value::Null, json!(true), json!(false)));
        assert_eq!((dashboard["page"].clone(), dashboard["pages"].clone()), (json!(1), json!(1)));
        assert_eq!(dashboard["profiles"][0]["id"], dashboard["profile"]["id"]);
        assert_eq!(dashboard["look"].as_str().unwrap().len(), 16);
        // A dark screen and an alert both show.
        crate::screen_off::set_screen(&wall, PadKind::Dashboard, false);
        crate::alerts::show(&shared, &wall, crate::alerts::from_command(&json!({"title": "Hi"})).unwrap());
        let after = state_message(&shared);
        assert_eq!((after["pads"][1]["screen_on"].clone(), after["pads"][1]["alert"].clone()), (json!(false), json!(true)));
        assert_ne!(after["pads"][1]["look"], dashboard["look"]);
        crate::screen_off::set_screen(&wall, PadKind::Dashboard, true);
    }

    #[test]
    fn the_state_says_how_each_pad_looks() {
        let mut cfg = Config::sample();
        cfg.style.shape = KeyShape::Rounded;
        cfg.style.corner = 17;
        let wall = cfg.add_dashboard_pad("Wall", 2, 4);
        cfg.pad_mut(&wall).unwrap().face = [255, 255, 255];
        let main = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        cfg.pad_mut(&main).unwrap().face_clear = true;
        cfg.repair();
        let (shared, _opened) = Shared::new(cfg, true);
        let state = state_message(&shared);
        let body = |at: usize| state["pads"][at]["body"].clone();
        assert_eq!(body(0)["front"], Value::Null, "a see-through front");
        assert_eq!(body(1)["front"], json!([255, 255, 255]));
        assert_eq!(body(1)["key_corner"], json!(0.2));
        assert!((body(1)["band_corner"].as_f64().unwrap() - 17.0 / 80.0).abs() < 0.001);
        assert_eq!((body(1)["edge"].clone(), body(1)["corner"].clone()), (json!(0.3), json!(0.3)));
        // Painting the front changes the look, so the pad's picture is fetched again.
        lock(&shared.cfg).pad_mut(&wall).unwrap().face = [0, 0, 0];
        assert_ne!(state_message(&shared)["pads"][1]["look"], state["pads"][1]["look"]);
    }
}
