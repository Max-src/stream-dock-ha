//! Icons for keys: Material Design Icons, the set Home Assistant itself uses,
//! so a key shows what the entity shows in Home Assistant.
//!
//! An icon is stored as `mdi:name`, the way Home Assistant writes them.

use crate::ha::{domain_of, Entity};
use ab_glyph::FontRef;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const FONT: &[u8] = include_bytes!("../assets/mdi.ttf");
const NAMES: &str = include_str!("../assets/mdi-names.txt");
/// For anything Home Assistant adds that nothing here knows about yet.
const LAST_RESORT: &str = "home-assistant";

/// Offered before anything has been searched for.
pub const COMMON: &[&str] = &[
    "thermometer", "water-percent", "lightbulb", "power", "fan", "air-conditioner", "radiator", "television",
    "speaker", "music", "play", "pause", "stop", "skip-next", "skip-previous", "volume-high", "volume-minus",
    "volume-off", "microphone", "home", "door", "window-closed", "garage", "lock", "shield-home", "bell",
    "camera", "cctv", "motion-sensor", "flash", "lightning-bolt", "power-plug", "battery", "solar-power",
    "fire", "snowflake", "weather-sunny", "weather-night", "weather-cloudy", "weather-rainy", "clock-outline",
    "timer-outline", "calendar", "wifi", "cog", "wrench", "robot-vacuum", "washing-machine", "car", "account",
    "heart", "star", "folder", "web", "keyboard", "monitor", "gamepad-variant", "email", "magnify", "delete",
];

/// Sensors and numbers, by what they measure.
const MEASURES: &[(&str, &str)] = &[
    ("apparent_power", "flash"),
    ("aqi", "air-filter"),
    ("atmospheric_pressure", "thermometer-lines"),
    ("battery", "battery"),
    ("carbon_dioxide", "molecule-co2"),
    ("carbon_monoxide", "molecule-co"),
    ("conductivity", "sprout-outline"),
    ("current", "current-ac"),
    ("data_rate", "transmission-tower"),
    ("data_size", "database"),
    ("date", "calendar"),
    ("distance", "arrow-left-right"),
    ("duration", "progress-clock"),
    ("energy", "lightning-bolt"),
    ("energy_storage", "car-battery"),
    ("frequency", "sine-wave"),
    ("gas", "meter-gas"),
    ("humidity", "water-percent"),
    ("illuminance", "brightness-5"),
    ("irradiance", "sun-wireless"),
    ("moisture", "water-percent"),
    ("monetary", "cash"),
    ("nitrogen_dioxide", "molecule"),
    ("nitrogen_monoxide", "molecule"),
    ("nitrous_oxide", "molecule"),
    ("ozone", "molecule"),
    ("ph", "ph"),
    ("pm1", "molecule"),
    ("pm10", "molecule"),
    ("pm25", "molecule"),
    ("pm4", "molecule"),
    ("power", "flash"),
    ("power_factor", "angle-acute"),
    ("precipitation", "weather-rainy"),
    ("precipitation_intensity", "weather-pouring"),
    ("pressure", "gauge"),
    ("reactive_power", "flash"),
    ("signal_strength", "wifi"),
    ("sound_pressure", "ear-hearing"),
    ("speed", "speedometer"),
    ("sulphur_dioxide", "molecule"),
    ("temperature", "thermometer"),
    ("timestamp", "clock-outline"),
    ("volatile_organic_compounds", "molecule"),
    ("voltage", "sine-wave"),
    ("volume", "cup-water"),
    ("volume_flow_rate", "waves-arrow-right"),
    ("water", "water"),
    ("weight", "weight"),
    ("wind_direction", "compass-outline"),
    ("wind_speed", "weather-windy"),
];

/// On/off sensors: what they mean, then the icon when on and when off.
const DETECTORS: &[(&str, &str, &str)] = &[
    ("battery", "battery-alert", "battery"),
    ("battery_charging", "battery-charging", "battery"),
    ("carbon_monoxide", "smoke-detector-alert", "smoke-detector"),
    ("cold", "snowflake", "thermometer"),
    ("connectivity", "check-network-outline", "close-network-outline"),
    ("door", "door-open", "door-closed"),
    ("garage_door", "garage-open", "garage"),
    ("gas", "alert-circle", "check-circle"),
    ("heat", "fire", "thermometer"),
    ("light", "brightness-7", "brightness-5"),
    ("lock", "lock-open", "lock"),
    ("moisture", "water", "water-off"),
    ("motion", "motion-sensor", "motion-sensor-off"),
    ("moving", "cursor-move", "cursor-default"),
    ("occupancy", "home", "home-outline"),
    ("opening", "square-outline", "square"),
    ("plug", "power-plug", "power-plug-off"),
    ("power", "power", "power-off"),
    ("presence", "home", "home-outline"),
    ("problem", "alert-circle", "check-circle"),
    ("running", "play", "stop"),
    ("safety", "alert-circle", "check-circle"),
    ("smoke", "smoke-detector-alert", "smoke-detector"),
    ("sound", "music-note", "music-note-off"),
    ("tamper", "alert-circle", "check-circle"),
    ("update", "package-up", "package"),
    ("vibration", "vibrate", "crop-portrait"),
    ("window", "window-open", "window-closed"),
];

/// Types of entity: the icon when on, and when off.
const TYPES: &[(&str, &str, &str)] = &[
    ("ai_task", "creation", "creation"),
    ("air_quality", "air-filter", "air-filter"),
    ("alarm_control_panel", "shield-home", "shield-off"),
    ("alert", "alert", "alert"),
    ("assist_satellite", "account-voice", "account-voice"),
    ("automation", "robot", "robot-off"),
    ("binary_sensor", "checkbox-marked-circle", "radiobox-blank"),
    ("button", "gesture-tap-button", "gesture-tap-button"),
    ("calendar", "calendar", "calendar"),
    ("camera", "cctv", "cctv"),
    ("climate", "thermostat", "thermostat"),
    ("conversation", "forum-outline", "forum-outline"),
    ("counter", "counter", "counter"),
    ("cover", "window-shutter-open", "window-shutter"),
    ("date", "calendar", "calendar"),
    ("datetime", "calendar-clock", "calendar-clock"),
    ("device_tracker", "account", "account-off"),
    ("event", "eye-check", "eye-check"),
    ("fan", "fan", "fan-off"),
    ("group", "google-circles-communities", "google-circles-communities"),
    ("humidifier", "air-humidifier", "air-humidifier-off"),
    ("image", "image", "image"),
    ("infrared", "remote", "remote"),
    ("input_boolean", "toggle-switch", "toggle-switch-off"),
    ("input_button", "gesture-tap-button", "gesture-tap-button"),
    ("input_datetime", "calendar-clock", "calendar-clock"),
    ("input_number", "ray-vertex", "ray-vertex"),
    ("input_select", "format-list-bulleted", "format-list-bulleted"),
    ("input_text", "form-textbox", "form-textbox"),
    ("lawn_mower", "robot-mower", "robot-mower"),
    ("light", "lightbulb", "lightbulb-outline"),
    ("lock", "lock-open", "lock"),
    ("media_player", "cast", "cast-off"),
    ("notify", "comment-alert", "comment-alert"),
    ("number", "ray-vertex", "ray-vertex"),
    ("person", "account", "account-off"),
    ("plant", "flower", "flower"),
    ("remote", "remote", "remote-off"),
    ("scene", "palette", "palette"),
    ("schedule", "calendar-clock", "calendar-clock"),
    ("script", "script-text", "script-text"),
    ("select", "format-list-bulleted", "format-list-bulleted"),
    ("sensor", "eye", "eye"),
    ("siren", "bullhorn", "bullhorn"),
    ("stt", "microphone-message", "microphone-message"),
    ("sun", "white-balance-sunny", "weather-night"),
    ("switch", "toggle-switch", "toggle-switch-off"),
    ("text", "form-textbox", "form-textbox"),
    ("time", "clock-outline", "clock-outline"),
    ("timer", "timer-outline", "timer-outline"),
    ("todo", "clipboard-list", "clipboard-list"),
    ("tts", "speaker-message", "speaker-message"),
    ("update", "package-up", "package"),
    ("vacuum", "robot-vacuum", "robot-vacuum"),
    ("valve", "pipe-valve", "pipe-valve"),
    ("wake_word", "chat-sleep", "chat-sleep"),
    ("water_heater", "water-boiler", "water-boiler-off"),
    ("weather", "weather-partly-cloudy", "weather-partly-cloudy"),
    ("zone", "map-marker-radius", "map-marker-radius"),
];

/// What a few types of entity are, beyond their type.
const KINDS: &[(&str, &str)] = &[
    ("button.identify", "crosshairs-question"),
    ("button.restart", "restart"),
    ("button.update", "package-up"),
    ("media_player.receiver", "audio-video"),
    ("media_player.speaker", "speaker"),
    ("media_player.tv", "television"),
    ("switch.outlet", "power-plug"),
    ("switch.switch", "toggle-switch"),
];

/// Weather entities by their state.
const WEATHER: &[(&str, &str)] = &[
    ("clear-night", "weather-night"),
    ("cloudy", "weather-cloudy"),
    ("exceptional", "alert-circle-outline"),
    ("fog", "weather-fog"),
    ("hail", "weather-hail"),
    ("lightning", "weather-lightning"),
    ("lightning-rainy", "weather-lightning-rainy"),
    ("partlycloudy", "weather-partly-cloudy"),
    ("pouring", "weather-pouring"),
    ("rainy", "weather-rainy"),
    ("snowy", "weather-snowy"),
    ("snowy-rainy", "weather-snowy-rainy"),
    ("sunny", "weather-sunny"),
    ("windy", "weather-windy"),
    ("windy-variant", "weather-windy-variant"),
];

/// Every icon: its name and its character, in name order.
fn all() -> &'static [(&'static str, char)] {
    static LIST: OnceLock<Vec<(&'static str, char)>> = OnceLock::new();
    LIST.get_or_init(|| {
        NAMES
            .lines()
            .filter_map(|line| {
                let (name, code) = line.split_once(' ')?;
                Some((name, char::from_u32(u32::from_str_radix(code, 16).ok()?)?))
            })
            .collect()
    })
}

fn by_name() -> &'static HashMap<&'static str, char> {
    static MAP: OnceLock<HashMap<&'static str, char>> = OnceLock::new();
    MAP.get_or_init(|| all().iter().copied().collect())
}

pub fn font() -> Option<&'static FontRef<'static>> {
    static LOADED: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    LOADED.get_or_init(|| FontRef::try_from_slice(FONT).ok()).as_ref()
}

fn bare_name(icon: &str) -> Option<&str> {
    icon.strip_prefix("mdi:").or_else(|| icon.strip_prefix("hass:"))
}

/// The character to draw for an icon written `mdi:name`.
pub fn character(icon: &str) -> Option<char> {
    by_name().get(bare_name(icon)?).copied()
}

/// How many icons there are to search.
pub fn count() -> usize {
    all().len()
}

/// Icons with every one of these words in their name, as (`mdi:name`, character).
/// With nothing to search for, the common ones.
pub fn search(words: &str, most: usize) -> Vec<(String, char)> {
    let words = words.to_lowercase();
    let words: Vec<&str> = words.split_whitespace().collect();
    if words.is_empty() {
        return COMMON.iter().filter_map(|name| Some((format!("mdi:{name}"), *by_name().get(name)?))).collect();
    }
    all()
        .iter()
        .filter(|(name, _)| words.iter().all(|word| name.contains(word)))
        .take(most)
        .map(|(name, c)| (format!("mdi:{name}"), *c))
        .collect()
}

fn on_off<'a>(on: &'a str, off: &'a str, is_on: bool) -> &'a str {
    if is_on {
        on
    } else {
        off
    }
}

/// A battery's icon shows how full it is.
fn battery_level(state: &str) -> Option<String> {
    let level = state.parse::<f32>().ok()?;
    Some(match level as i32 {
        i32::MIN..=9 => "battery-outline".to_string(),
        95.. => "battery".to_string(),
        level => format!("battery-{}", level / 10 * 10),
    })
}

fn measured(class: &str) -> Option<&'static str> {
    MEASURES.iter().find(|(name, _)| *name == class).map(|(_, icon)| *icon)
}

/// The icon Home Assistant would show for an entity: its own if it has one,
/// otherwise by what it measures, or else by its type. There is always one.
pub fn of_entity(id: &str, entity: Option<&Entity>) -> String {
    let own = entity.and_then(|e| e.attributes.get("icon")).and_then(Value::as_str);
    if let Some(own) = own.filter(|icon| character(icon).is_some()).and_then(bare_name) {
        return format!("mdi:{own}");
    }
    let domain = domain_of(id);
    let class = entity.map_or("", |e| e.device_class.as_str());
    let state = entity.map_or("", |e| e.state.as_str());
    let is_on = entity.and_then(Entity::is_on).unwrap_or(true);

    if matches!(domain, "sensor" | "number") && class == "battery" {
        if let Some(level) = battery_level(state) {
            return format!("mdi:{level}");
        }
    }
    let kind = format!("{domain}.{class}");
    let specific = match domain {
        "binary_sensor" => {
            DETECTORS.iter().find(|(name, ..)| *name == class).map(|(_, on, off)| on_off(on, off, state == "on"))
        }
        "sensor" | "number" | "input_number" => measured(class),
        "weather" => WEATHER.iter().find(|(name, _)| *name == state).map(|(_, icon)| *icon),
        _ => KINDS.iter().find(|(name, _)| *name == kind).map(|(_, icon)| *icon),
    };
    let general = || TYPES.iter().find(|(name, ..)| *name == domain).map(|(_, on, off)| on_off(on, off, is_on));
    format!("mdi:{}", specific.or_else(general).unwrap_or(LAST_RESORT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::{point, Font, PxScale};

    fn entity(state: &str, class: &str) -> Entity {
        Entity { state: state.into(), device_class: class.into(), ..Entity::default() }
    }

    fn draws(icon: &str) -> bool {
        let c = character(icon).unwrap_or_else(|| panic!("{icon} isn't in the icon set"));
        let font = font().unwrap();
        font.outline_glyph(font.glyph_id(c).with_scale_and_position(PxScale::from(40.0), point(0.0, 0.0))).is_some()
    }

    #[test]
    fn every_icon_this_app_can_choose_really_draws() {
        let tables = MEASURES.iter().chain(KINDS).chain(WEATHER).map(|(_, icon)| *icon);
        let both = DETECTORS.iter().chain(TYPES).flat_map(|(_, on, off)| [*on, *off]);
        for name in tables.chain(both).chain(COMMON.iter().copied()).chain([LAST_RESORT]) {
            assert!(draws(&format!("mdi:{name}")), "{name}");
        }
        for level in (10..=90).step_by(10) {
            assert!(draws(&format!("mdi:battery-{level}")));
        }
        assert!(draws("mdi:battery-outline"));
    }

    #[test]
    fn the_whole_set_is_loaded() {
        assert!(count() > 7000);
        let font = font().unwrap();
        let lost: Vec<_> = all().iter().filter(|(_, c)| font.glyph_id(*c).0 == 0).collect();
        assert!(lost.is_empty(), "{lost:?}");
    }

    #[test]
    fn a_temperature_sensor_gets_a_thermometer() {
        assert_eq!(of_entity("sensor.bedroom", Some(&entity("21.5", "temperature"))), "mdi:thermometer");
    }

    #[test]
    fn an_entitys_own_icon_comes_first() {
        let mut fan = entity("on", "");
        fan.attributes.insert("icon".into(), "mdi:fan-chevron-up".into());
        assert_eq!(of_entity("fan.ceiling", Some(&fan)), "mdi:fan-chevron-up");
        fan.attributes.insert("icon".into(), "hass:thermostat".into());
        assert_eq!(of_entity("fan.ceiling", Some(&fan)), "mdi:thermostat");
        // An icon from an add-on set can't be drawn, so the usual one stands in.
        fan.attributes.insert("icon".into(), "phu:rooms".into());
        assert_eq!(of_entity("fan.ceiling", Some(&fan)), "mdi:fan");
    }

    #[test]
    fn icons_follow_the_state() {
        assert_eq!(of_entity("binary_sensor.front", Some(&entity("on", "door"))), "mdi:door-open");
        assert_eq!(of_entity("binary_sensor.front", Some(&entity("off", "door"))), "mdi:door-closed");
        assert_eq!(of_entity("light.desk", Some(&entity("off", ""))), "mdi:lightbulb-outline");
        assert_eq!(of_entity("lock.front", Some(&entity("locked", ""))), "mdi:lock");
        assert_eq!(of_entity("weather.home", Some(&entity("rainy", ""))), "mdi:weather-rainy");
        assert_eq!(of_entity("sensor.phone", Some(&entity("54", "battery"))), "mdi:battery-50");
        assert_eq!(of_entity("sensor.phone", Some(&entity("4", "battery"))), "mdi:battery-outline");
        assert_eq!(of_entity("sensor.phone", Some(&entity("100", "battery"))), "mdi:battery");
        assert_eq!(of_entity("sensor.phone", Some(&entity("unavailable", "battery"))), "mdi:battery");
    }

    /// The types and kinds on the person's own Home Assistant, as listed on 2026-11-01.
    #[test]
    fn every_kind_of_entity_seen_at_home_has_a_fitting_icon() {
        let kinds = [
            ("ai_task", ""), ("alarm_control_panel", ""), ("automation", ""), ("binary_sensor", ""),
            ("binary_sensor", "battery"), ("binary_sensor", "connectivity"), ("binary_sensor", "door"),
            ("binary_sensor", "heat"), ("binary_sensor", "motion"), ("binary_sensor", "occupancy"),
            ("binary_sensor", "plug"), ("binary_sensor", "problem"), ("binary_sensor", "running"),
            ("button", ""), ("button", "identify"), ("button", "restart"), ("camera", ""), ("climate", ""),
            ("conversation", ""), ("device_tracker", ""), ("event", ""), ("event", "button"), ("fan", ""),
            ("infrared", "emitter"), ("infrared", "receiver"), ("input_boolean", ""), ("input_button", ""),
            ("input_datetime", ""), ("input_number", ""), ("input_select", ""), ("input_text", ""), ("light", ""),
            ("media_player", ""), ("media_player", "speaker"), ("media_player", "tv"), ("notify", ""), ("number", ""),
            ("number", "power"), ("number", "temperature"), ("person", ""), ("remote", ""), ("scene", ""), ("script", ""),
            ("select", ""), ("sensor", ""), ("sensor", "aqi"), ("sensor", "atmospheric_pressure"), ("sensor", "battery"),
            ("sensor", "current"), ("sensor", "data_rate"), ("sensor", "data_size"), ("sensor", "duration"),
            ("sensor", "energy"), ("sensor", "enum"), ("sensor", "humidity"), ("sensor", "illuminance"),
            ("sensor", "moisture"), ("sensor", "power"), ("sensor", "signal_strength"), ("sensor", "temperature"),
            ("sensor", "timestamp"), ("sensor", "voltage"), ("sensor", "weight"), ("stt", ""), ("sun", ""),
            ("switch", ""), ("switch", "outlet"), ("switch", "switch"), ("text", ""), ("time", ""), ("tts", ""),
            ("update", ""), ("update", "firmware"), ("wake_word", ""), ("weather", ""), ("zone", ""),
        ];
        for (domain, class) in kinds {
            let icon = of_entity(&format!("{domain}.x"), Some(&entity("on", class)));
            assert!(draws(&icon), "{domain} {class}");
            assert_ne!(icon, format!("mdi:{LAST_RESORT}"), "{domain} {class}");
        }
        assert!(draws(&of_entity("new_kind_of_thing.x", None)));
    }

    #[test]
    fn search_finds_by_any_part_of_the_name() {
        let found = search("therm", 500);
        assert!(found.iter().any(|(name, _)| name == "mdi:thermometer"));
        let found = search("fan off", 500);
        assert!(found.iter().any(|(name, _)| name == "mdi:fan-off"));
        assert!(found.iter().all(|(name, _)| name.contains("fan") && name.contains("off")));
        assert!(search("zzzzz", 500).is_empty());
        assert_eq!(search("  ", 500).len(), COMMON.len());
        assert_eq!(search("a", 40).len(), 40);
    }
}
