//! Pictures of the pads that Home Assistant's dashboard cards show
//! (docs/integration-protocol.md, sections 3.4 and 4.2): JPEGs of each screen
//! control at the size the cards want, sent when they change.

use crate::band::Face;
use crate::frames::{self, PadFrame};
use crate::model::{lock, Shared};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// At most ten sends a second for each pad.
const LEAST_GAP: Duration = Duration::from_millis(100);
/// A message stays under 1 MB of JSON, with room for its other fields.
const MOST_BYTES: usize = 900_000;
const QUALITY: u8 = 90;

/// What one watched pad has been sent.
#[derive(Debug, Default)]
struct Watched {
    scale: u32,
    layout: String,
    /// Each control's picture as last sent: its hash, or None for nothing.
    sent: HashMap<String, Option<u64>>,
    /// The next send holds every screen control.
    complete: bool,
    /// Something may have changed since the last send.
    dirty: bool,
    last: Option<Instant>,
}

/// The pads being watched, by id.
#[derive(Debug, Default)]
pub struct Watching {
    pads: BTreeMap<String, Watched>,
}

impl Watching {
    /// Forgets everything: nothing is watched until the integration says.
    pub fn reset(&mut self) {
        self.pads.clear();
    }

    /// Takes the integration's full list of watched pads and their scales.
    /// A pad new to it, or at a new scale, gets a complete set at once.
    pub fn watch(&mut self, wanted: &Map<String, Value>) {
        let mut pads = BTreeMap::new();
        for (pad_id, scale) in wanted {
            let scale = scale.as_u64().unwrap_or(2).clamp(1, 3) as u32;
            let kept = self.pads.remove(pad_id).filter(|watched| watched.scale == scale);
            let watched = kept.unwrap_or(Watched { scale, complete: true, dirty: true, ..Watched::default() });
            pads.insert(pad_id.clone(), watched);
        }
        self.pads = pads;
    }

    /// The pads being watched.
    pub fn pad_ids(&self) -> Vec<String> {
        self.pads.keys().cloned().collect()
    }

    /// A pad that got a new id isn't watched under its old one.
    pub fn forget(&mut self, pad_id: &str) {
        self.pads.remove(pad_id);
    }

    /// Something may look different now.
    pub fn changed(&mut self) {
        for watched in self.pads.values_mut() {
            watched.dirty = true;
        }
    }

    /// When the next send is allowed for a pad that's waiting.
    pub fn next_due(&self) -> Option<Instant> {
        let waiting = self.pads.values().filter(|watched| watched.dirty);
        waiting.map(|watched| watched.last.map_or_else(Instant::now, |last| last + LEAST_GAP)).min()
    }

    /// The `pictures` messages due now.
    pub fn due(&mut self, shared: &Shared) -> Vec<Value> {
        let now = Instant::now();
        let mut messages = Vec::new();
        for (pad_id, watched) in self.pads.iter_mut() {
            if !watched.dirty || watched.last.is_some_and(|last| now < last + LEAST_GAP) {
                continue;
            }
            watched.dirty = false;
            let sent = watched.update(shared, pad_id);
            if !sent.is_empty() {
                watched.last = Some(now);
            }
            messages.extend(sent);
        }
        messages
    }
}

/// Where a control's picture comes from in a frame.
enum Part {
    Cell(usize),
    Band(usize),
}

impl Watched {
    /// The messages that bring the integration up to date with this pad.
    fn update(&mut self, shared: &Shared, pad_id: &str) -> Vec<Value> {
        // Worked out with the settings held, and drawn once they're let go.
        let (frame, dark) = {
            let cfg = lock(&shared.cfg);
            let entities = lock(&shared.ha.entities);
            let Some(frame) = frames::pad_frame(shared, &cfg, &entities, pad_id) else { return Vec::new() };
            (frame, crate::screen_off::pad_asleep(&cfg, pad_id))
        };
        if frame.layout.version != self.layout {
            self.layout = frame.layout.version.clone();
            self.complete = true;
        }
        let complete = std::mem::take(&mut self.complete);
        let mut wanted: Vec<(String, Option<u64>, Part)> = Vec::new();
        for (index, shown) in frame.cells.iter().enumerate() {
            let hash = shown.look.as_ref().filter(|_| !dark).map(|_| shown.hash);
            wanted.push((shown.control.clone(), hash, Part::Cell(index)));
        }
        for (index, (name, face, hash)) in frame.band.iter().enumerate() {
            let hash = (!dark && *face != Face::default()).then_some(*hash);
            wanted.push((name.clone(), hash, Part::Band(index)));
        }
        if complete {
            self.sent.clear();
        }
        let changed: Vec<(String, Option<u64>, Part)> =
            wanted.into_iter().filter(|(name, hash, _)| complete || self.sent.get(name) != Some(hash)).collect();
        if changed.is_empty() && !complete {
            return Vec::new();
        }
        let mut entries = Vec::new();
        for (name, hash, part) in changed {
            let picture = hash.and_then(|hash| Some(json!({ "hash": format!("{hash:016x}"), "data": encoded(&frame, &part, self.scale)? })));
            self.sent.insert(name.clone(), hash);
            entries.push((name, picture.unwrap_or(Value::Null)));
        }
        split(pad_id, self.scale, complete, entries, MOST_BYTES)
    }
}

/// A control's picture as base64 JPEG.
fn encoded(frame: &PadFrame, part: &Part, scale: u32) -> Option<String> {
    let picture = match part {
        Part::Cell(index) => frames::cell_picture(frame.cells.get(*index)?, scale),
        Part::Band(index) => frames::band_picture(frame, *index, scale)?,
    };
    Some(base64(&jpeg(&picture)?))
}

/// Puts the pictures into `pictures` messages of at most `most` bytes each,
/// as near as can be told. Only the first says `complete`.
fn split(pad_id: &str, scale: u32, complete: bool, entries: Vec<(String, Value)>, most: usize) -> Vec<Value> {
    let message = |pictures: Map<String, Value>, first: bool| {
        json!({ "type": "stream_dock_ha/pictures", "pad_id": pad_id, "scale": scale, "complete": complete && first, "pictures": pictures })
    };
    let mut messages = Vec::new();
    let (mut part, mut size) = (Map::new(), 0);
    for (name, picture) in entries {
        let cost = name.len() + picture.get("data").and_then(Value::as_str).map_or(0, str::len) + 64;
        if !part.is_empty() && size + cost > most {
            messages.push(message(std::mem::take(&mut part), messages.is_empty()));
            size = 0;
        }
        part.insert(name, picture);
        size += cost;
    }
    if !part.is_empty() || messages.is_empty() {
        messages.push(message(part, messages.is_empty()));
    }
    messages
}

/// A picture as a JPEG at the protocol's quality.
pub(crate) fn jpeg(picture: &image::RgbImage) -> Option<Vec<u8>> {
    let (width, height) = (picture.width() as usize, picture.height() as usize);
    crate::monitor::rgb_jpeg(picture.as_raw(), width, height, QUALITY, jpeg_encoder::SamplingFactor::F_1_1)
}

/// Standard base64, with padding.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let joined = chunk.iter().enumerate().fold(0u32, |all, (at, byte)| all | ((*byte as u32) << (16 - 8 * at)));
        for at in 0..4 {
            if at <= chunk.len() {
                text.push(LETTERS[((joined >> (18 - 6 * at)) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// A picture of the whole pad, as the `snapshot` command answers: laid out
/// as in its layout, on its front colour.
pub fn snapshot(shared: &Shared, pad_id: &str, scale: u32) -> Result<Value, String> {
    let cfg = lock(&shared.cfg).clone();
    let frame = {
        let entities = lock(&shared.ha.entities);
        frames::pad_frame(shared, &cfg, &entities, pad_id)
    };
    let frame = frame.ok_or("That pad isn't in use yet")?;
    let picture = frames::pad_snapshot(&cfg, pad_id, &frame, scale.clamp(1, 3));
    let data = jpeg(&picture).ok_or("The picture is too big")?;
    Ok(json!({ "content_type": "image/jpeg", "data": base64(&data) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_the_standard_kind() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn a_big_update_is_split_with_complete_only_on_the_first() {
        let picture = |n: usize| json!({ "hash": "0", "data": "x".repeat(n) });
        let entries = vec![("key_1".into(), picture(400)), ("key_2".into(), picture(400)), ("key_3".into(), Value::Null), ("key_4".into(), picture(400))];
        let messages = split("0123456789abcdef", 2, true, entries, 1000);
        assert_eq!(messages.len(), 2);
        assert_eq!((messages[0]["complete"].clone(), messages[1]["complete"].clone()), (json!(true), json!(false)));
        assert_eq!(messages[0]["pictures"].as_object().unwrap().len(), 2);
        assert!(messages[1]["pictures"]["key_3"].is_null() && messages[1]["pictures"]["key_4"].is_object());
        assert!(messages.iter().all(|message| message.to_string().len() < 1000 + 200));
        // Nothing changed but a complete set was asked for: it still goes, empty.
        let empty = split("0123456789abcdef", 1, true, Vec::new(), 1000);
        assert_eq!((empty.len(), empty[0]["complete"].clone()), (1, json!(true)));
    }

    #[test]
    fn jpegs_come_out_at_the_size_drawn() {
        let data = jpeg(&image::RgbImage::from_pixel(170, 170, image::Rgb([200, 30, 30]))).unwrap();
        let back = image::load_from_memory(&data).unwrap();
        assert_eq!((back.width(), back.height()), (170, 170));
    }
}
