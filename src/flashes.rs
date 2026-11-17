//! Something from Home Assistant on one key for a while (`show_on_key`): words,
//! an icon, a colour, a camera or a picture, over whatever that key shows on
//! the pad's running profile, whatever page it's on. Several keys can show
//! one at once. Pressing that key takes it away and does nothing else; the
//! other keys work as usual. A whole-pad alert covers flashes too. Like
//! alerts, they're only kept in memory.

use crate::model::{lock, KeyStyle, PictureFit, Shared};
use crate::render::{CameraPart, Look};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a flash stays when the command doesn't say.
pub const SECONDS: f64 = 30.0;

/// One key's flash.
#[derive(Clone, Debug, PartialEq)]
pub struct Flash {
    pub title: String,
    pub message: String,
    pub icon: String,
    pub color: [u8; 3],
    /// A camera whose live picture fills the key.
    pub camera: Option<String>,
    /// Where its picture comes from, as the command said: an `image.*`
    /// entity, a path on Home Assistant, a web address or a `data:` address.
    pub picture: Option<String>,
    pub fit: PictureFit,
    /// When it goes away by itself, if it does.
    pub until: Option<Instant>,
    pub serial: u64,
}

/// The flashes showing, by pad id and cell.
pub type Flashes = HashMap<(String, usize), Flash>;

/// The flash a `show_on_key` command asks for, or what's wrong with it.
pub fn from_command(data: &Value) -> Result<Flash, String> {
    let text = |field: &str| -> Result<String, String> {
        match data.get(field) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(text)) => Ok(text.trim().to_string()),
            Some(_) => Err(format!("\"{field}\" should be text")),
        }
    };
    let color = match data.get("color") {
        None | Some(Value::Null) => crate::alerts::AMBER,
        Some(value) => serde_json::from_value::<[u8; 3]>(value.clone())
            .map_err(|_| "\"color\" should be red, green and blue, like [255, 170, 0]".to_string())?,
    };
    let seconds = match data.get("seconds") {
        None | Some(Value::Null) => SECONDS,
        Some(value) => value.as_f64().filter(|seconds| *seconds >= 0.0).ok_or("\"seconds\" should be a number, 0 or more")?,
    };
    let camera = Some(text("camera")?).filter(|camera| !camera.is_empty());
    if camera.as_deref().is_some_and(|camera| !camera.starts_with("camera.")) {
        return Err("\"camera\" should be a camera entity, like camera.front_door".into());
    }
    let picture = Some(text("picture")?).filter(|picture| !picture.is_empty());
    if let Some(picture) = &picture {
        if camera.is_some() {
            return Err("Choose \"camera\" or \"picture\", not both".into());
        }
        let known = ["image.", "/", "http://", "https://", "data:image/"];
        if !known.iter().any(|start| picture.starts_with(start)) {
            return Err("\"picture\" should be an image entity (image.door), a path on Home Assistant (/local/door.png), a web address or a data: address".into());
        }
    }
    let fit = match text("fit")?.as_str() {
        "" | "fill" => PictureFit::Fill,
        "fit" => PictureFit::Fit,
        "stretch" => PictureFit::Stretch,
        other => return Err(format!("\"fit\" should be fill, fit or stretch, not \"{other}\"")),
    };
    Ok(Flash {
        title: text("title")?,
        message: text("message")?,
        icon: text("icon")?,
        color,
        camera,
        picture,
        fit,
        until: (seconds > 0.0).then(|| Instant::now() + Duration::from_secs_f64(seconds.min(86_400.0))),
        serial: 0,
    })
}

/// The name a flash's picture is kept under, with the camera pictures.
fn picture_name(pad_id: &str, cell: usize) -> String {
    format!("flash:{pad_id}:{cell}")
}

/// The timer's name for a flash, which can't be taken for a pad's id.
fn timer_name(pad_id: &str, cell: usize) -> String {
    format!("flash\n{pad_id}\n{cell}")
}

/// Puts a flash on a key, in place of any it had, once its picture (if it has
/// one) is fetched: a picture that can't be had is refused rather than shown
/// as an empty key.
pub fn show(shared: &Arc<Shared>, pad_id: &str, cell: usize, mut flash: Flash) -> Result<(), String> {
    if let Some(source) = &flash.picture {
        let picture = fetch_picture(shared, source)?;
        crate::camera::store(&picture_name(pad_id, cell), picture);
    }
    flash.serial = crate::alerts::next_serial();
    let (serial, until) = (flash.serial, flash.until);
    lock(&shared.flashes).insert((pad_id.to_string(), cell), flash);
    changed(shared);
    if let Some(until) = until {
        crate::alerts::take_away_at(crate::alerts::Timed {
            shared: Arc::downgrade(shared),
            name: timer_name(pad_id, cell),
            serial,
            until,
            remove: remove_timed,
        });
    }
    Ok(())
}

/// Takes a timed flash away, unless another has taken its place.
fn remove_timed(shared: &Shared, name: &str, serial: u64) {
    let mut parts = name.split('\n').skip(1);
    let (Some(pad_id), Some(Ok(cell))) = (parts.next(), parts.next().map(str::parse::<usize>)) else { return };
    let key = (pad_id.to_string(), cell);
    let mut flashes = lock(&shared.flashes);
    if flashes.get(&key).is_some_and(|flash| flash.serial == serial) {
        flashes.remove(&key);
        drop(flashes);
        crate::camera::forget(&picture_name(pad_id, cell));
        changed(shared);
    }
}

/// Takes the flash off one key, and says whether it had one.
pub fn clear_key(shared: &Shared, pad_id: &str, cell: usize) -> bool {
    let removed = lock(&shared.flashes).remove(&(pad_id.to_string(), cell)).is_some();
    if removed {
        crate::camera::forget(&picture_name(pad_id, cell));
        changed(shared);
    }
    removed
}

/// Takes the flashes off one key of a pad, or all its keys, and says how many it took.
pub fn clear(shared: &Shared, pad_id: &str, cell: Option<usize>) -> usize {
    if let Some(cell) = cell {
        return clear_key(shared, pad_id, cell) as usize;
    }
    let cells: Vec<usize> = lock(&shared.flashes).keys().filter(|(pad, _)| pad == pad_id).map(|(_, cell)| *cell).collect();
    cells.iter().filter(|cell| clear_key(shared, pad_id, **cell)).count()
}

fn changed(shared: &Shared) {
    crate::pad::wake();
    shared.touch();
}

/// The flash showing on a key now, if any.
#[cfg(test)]
pub fn active(shared: &Shared, pad_id: &str, cell: usize) -> Option<Flash> {
    let flashes = lock(&shared.flashes);
    flashes.get(&(pad_id.to_string(), cell)).filter(|flash| flash.until.is_none_or(|until| until > Instant::now())).cloned()
}

/// Every flash showing on a pad now, by cell.
pub fn on_pad(shared: &Shared, pad_id: &str) -> Vec<(usize, Flash)> {
    let now = Instant::now();
    let flashes = lock(&shared.flashes);
    flashes
        .iter()
        .filter(|((pad, _), flash)| pad == pad_id && flash.until.is_none_or(|until| until > now))
        .map(|((_, cell), flash)| (*cell, flash.clone()))
        .collect()
}

/// The cameras flashes are showing, so their pictures are fetched too.
pub fn cameras(shared: &Shared) -> Vec<String> {
    lock(&shared.flashes).values().filter_map(|flash| flash.camera.clone()).collect()
}

/// How a flash looks on its key, in the pad's key style. `outside` is the
/// colour round the key's shape.
pub fn look(flash: &Flash, pad_id: &str, cell: usize, style: &KeyStyle, outside: [u8; 3]) -> Look {
    let shown = flash.camera.clone().or_else(|| flash.picture.as_ref().map(|_| picture_name(pad_id, cell)));
    let camera = shown.map(|entity| CameraPart {
        version: crate::camera::version(&entity),
        entity,
        at: [0, 0],
        span: [1, 1],
        fit: flash.fit,
        gaps: [0, 0],
        shape: style.shape,
        corner: style.corner,
        outside,
    });
    let text = [flash.title.as_str(), flash.message.as_str()].into_iter().filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
    // Something always shows: without words or a picture, a bell.
    let glyph = match (&camera, flash.icon.is_empty()) {
        (Some(_), _) => String::new(),
        (None, false) => flash.icon.clone(),
        (None, true) if text.is_empty() => crate::alerts::BELL.to_string(),
        (None, true) => String::new(),
    };
    Look {
        text,
        glyph,
        color: flash.color,
        image: None,
        image_changed: None,
        fit: flash.fit,
        style: style.clone(),
        toggle: None,
        track: None,
        camera,
        graph: None,
        unavailable: false,
    }
}

/// Where a picture is fetched from, and whether Home Assistant's token goes
/// with it: only to Home Assistant's own address, never to another site.
fn picture_address(source: &str, base: &str) -> Result<(String, bool), String> {
    let base = base.trim_end_matches('/');
    if let Some(entity) = source.strip_prefix("image.") {
        return Ok((format!("{base}/api/image_proxy/image.{entity}"), true));
    }
    if source.starts_with('/') {
        return Ok((format!("{base}{source}"), true));
    }
    let own = !base.is_empty() && (source == base || source.starts_with(&format!("{base}/")));
    Ok((source.to_string(), own))
}

/// Fetches a flash's picture, or decodes it from a `data:` address.
fn fetch_picture(shared: &Shared, source: &str) -> Result<image::RgbImage, String> {
    if let Some(data) = source.strip_prefix("data:") {
        let (_, encoded) = data.split_once(";base64,").ok_or("A data: picture should be base64, like data:image/png;base64,...")?;
        let bytes = decode_base64(encoded).ok_or("That data: picture isn't valid base64")?;
        return image::load_from_memory(&bytes).map(|picture| picture.to_rgb8()).map_err(|e| format!("That picture can't be read: {e}"));
    }
    let (base, token) = {
        let cfg = lock(&shared.cfg);
        (crate::ha::rest_address(&cfg.ha_url), crate::ha::unprotect(&cfg.ha_token).unwrap_or_default())
    };
    let (address, own) = picture_address(source, &base)?;
    let token = Some(token.as_str()).filter(|token| own && !token.is_empty());
    crate::camera::fetch_with(&address, token).map_err(|problem| format!("Couldn't get the picture {source}: {problem}"))
}

/// Standard base64, as a data: address carries it; spaces and line breaks are skipped.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        bits = (bits << 6) | value as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, PadKind};
    use serde_json::json;

    #[test]
    fn a_command_becomes_a_flash_with_sensible_defaults() {
        let flash = from_command(&json!({"title": "Door"})).unwrap();
        assert_eq!((flash.title.as_str(), flash.color, flash.fit), ("Door", crate::alerts::AMBER, PictureFit::Fill));
        let until = flash.until.unwrap();
        assert!(until > Instant::now() + Duration::from_secs(29) && until <= Instant::now() + Duration::from_secs(30));
        assert_eq!(from_command(&json!({"seconds": 0})).unwrap().until, None, "0 stays until pressed");
        assert!(from_command(&json!({"color": "red"})).is_err());
        assert!(from_command(&json!({"seconds": -1})).is_err());
        assert!(from_command(&json!({"camera": "light.desk"})).is_err());
        assert!(from_command(&json!({"picture": "door.png"})).is_err());
        assert!(from_command(&json!({"camera": "camera.door", "picture": "/local/a.png"})).is_err());
        assert!(from_command(&json!({"fit": "zoom"})).is_err());
        for picture in ["image.door", "/local/door.png", "https://example.com/a.png", "data:image/png;base64,AAAA"] {
            assert!(from_command(&json!({ "picture": picture })).is_ok(), "{picture}");
        }
    }

    #[test]
    fn the_token_only_goes_to_home_assistant() {
        let base = "http://192.168.1.2:8123";
        assert_eq!(picture_address("image.door", base).unwrap(), (format!("{base}/api/image_proxy/image.door"), true));
        assert_eq!(picture_address("/local/a.png", base).unwrap(), (format!("{base}/local/a.png"), true));
        assert_eq!(picture_address(&format!("{base}/local/a.png"), base).unwrap().1, true);
        assert_eq!(picture_address("https://example.com/a.png", base).unwrap().1, false);
        // A look-alike address isn't Home Assistant's.
        assert_eq!(picture_address("http://192.168.1.2:8123.evil.com/a.png", base).unwrap().1, false);
    }

    #[test]
    fn base64_decodes_as_data_addresses_carry_it() {
        assert_eq!(decode_base64("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(decode_base64("SGVs\nbG8h").unwrap(), b"Hello!");
        assert!(decode_base64("not*base64").is_none());
    }

    #[test]
    fn a_data_picture_is_shown_on_its_key() {
        let (shared, _opened) = Shared::new(Config::default(), true);
        let pad = lock(&shared.cfg).pads[0].id.clone();
        let mut png = Vec::new();
        image::RgbImage::from_pixel(8, 8, image::Rgb([0, 200, 0]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let source = format!("data:image/png;base64,{}", crate::bridge::base64(&png));
        show(&shared, &pad, 3, from_command(&json!({ "picture": source, "title": "" })).unwrap()).unwrap();
        let flash = active(&shared, &pad, 3).unwrap();
        let look = look(&flash, &pad, 3, &KeyStyle::default(), [0, 0, 0]);
        let picture = crate::render::key_picture(&look);
        assert_eq!(picture.get_pixel(42, 42).0, [0, 200, 0]);
        assert!(clear_key(&shared, &pad, 3));
        assert_eq!(crate::camera::version(&picture_name(&pad, 3)), 0, "its picture goes with it");
        // A picture that can't be had isn't shown.
        assert!(show(&shared, &pad, 3, from_command(&json!({"picture": "data:image/png;base64,AAAA"})).unwrap()).is_err());
        assert!(active(&shared, &pad, 3).is_none());
    }

    #[test]
    fn flashes_sit_on_their_keys_until_cleared_or_their_time_is_up() {
        let (shared, _opened) = Shared::new(Config::default(), true);
        let pad = lock(&shared.cfg).pads[0].id.clone();
        show(&shared, &pad, 0, from_command(&json!({"title": "One", "seconds": 0})).unwrap()).unwrap();
        show(&shared, &pad, 0, from_command(&json!({"title": "Two", "seconds": 0})).unwrap()).unwrap();
        show(&shared, &pad, 1, from_command(&json!({"title": "Soon gone", "seconds": 0.2})).unwrap()).unwrap();
        show(&shared, &pad, 2, from_command(&json!({"title": "Three", "seconds": 0})).unwrap()).unwrap();
        assert_eq!(active(&shared, &pad, 0).unwrap().title, "Two", "a new one replaces the old");
        assert_eq!(on_pad(&shared, &pad).len(), 3);
        std::thread::sleep(Duration::from_millis(400));
        assert!(active(&shared, &pad, 1).is_none());
        assert_eq!(lock(&shared.flashes).len(), 2, "taken away, not only hidden");
        assert_eq!(clear(&shared, &pad, Some(2)), 1);
        assert_eq!(clear(&shared, &pad, None), 1);
        assert!(lock(&shared.flashes).is_empty());
    }

    #[test]
    fn a_flash_shows_on_its_key_on_the_pad_and_an_alert_covers_it() {
        let mut cfg = Config::default();
        cfg.add_small_pad("Small");
        cfg.repair();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let main = cfg.pads[0].id.clone();
        let entities = crate::ha::Entities::new();
        show(&shared, &main, 4, from_command(&json!({"title": "Door", "color": [0, 0, 200]})).unwrap()).unwrap();
        let frame = crate::frames::pad_frame(&shared, &cfg, &entities, &main).unwrap();
        let look = frame.look_of(4).unwrap();
        assert_eq!((look.text.as_str(), look.color), ("Door", [0, 0, 200]));
        // Only on that pad.
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        let other = crate::frames::pad_frame(&shared, &cfg, &entities, &small).unwrap();
        assert!(other.look_of(4).is_none_or(|look| look.text != "Door"));
        crate::alerts::show(&shared, &main, crate::alerts::from_command(&json!({"title": "Alert"})).unwrap());
        let frame = crate::frames::pad_frame(&shared, &cfg, &entities, &main).unwrap();
        assert!(frame.look_of(4).unwrap().is_stretched(), "the alert is on top");
        crate::alerts::clear(&shared, &main);
    }

    /// A flash on a 293S key, with words, with an icon and words, and with a
    /// long message. With `STREAM_DOCK_HA_SHOT_FLASHES` naming a folder, the
    /// pictures are saved there to look at.
    #[test]
    fn a_flash_reads_on_one_key() {
        let folder = std::env::var_os("STREAM_DOCK_HA_SHOT_FLASHES").map(std::path::PathBuf::from);
        let cases = [
            ("title", json!({"title": "Door"})),
            ("icon-title", json!({"title": "Door", "icon": "mdi:doorbell"})),
            ("title-message", json!({"title": "Washer", "message": "Done in 12 min", "color": [30, 120, 220]})),
            ("bell", json!({})),
        ];
        for (name, command) in cases {
            let flash = from_command(&command).unwrap();
            let picture = crate::render::key_picture_sized(&look(&flash, "pad", 0, &KeyStyle::default(), [0, 0, 0]), 170);
            assert_eq!(picture.dimensions(), (170, 170));
            if let Some(folder) = &folder {
                picture.save(folder.join(format!("flash-{name}.png"))).unwrap();
            }
        }
    }
}
