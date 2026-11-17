//! Mini Monitor on dashboard cards: the whole picture a pad's Mini Monitor
//! shows, sent as one JPEG per frame with where each key's part of it is
//! (docs/integration-protocol.md, section 3.6). Frames go only for pads a
//! card is watching, while their Mini Monitor is on, at most
//! `Config::ha_monitor_fps` a second.
//!
//! When Home Assistant has a local address other than the one the app uses,
//! frames go over a connection of their own to it, so they stay on the home
//! network; while that connection is down they go the usual way.

use crate::ha::websocket_address;
use crate::model::layout::layout_of;
use crate::model::{lock, small_cell_of_key, Config, PadKind, Shared};
use crate::monitor::Shown;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

/// How good the frames' JPEGs are: a moving screen hides what this loses.
const QUALITY: u8 = 70;
/// How long the local connection waits before trying again.
const LOCAL_RETRY: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(30) };
/// How long connecting to the local address may take.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(4);

/// What the integration gave for frames when the app connected.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Grant {
    pub key: String,
    pub local_url: Option<String>,
}

impl Grant {
    pub fn of(result: &Value) -> Grant {
        Grant {
            key: result.get("frame_key").and_then(Value::as_str).unwrap_or_default().to_string(),
            local_url: result.get("local_url").and_then(Value::as_str).filter(|url| !url.trim().is_empty()).map(str::to_string),
        }
    }
}

/// The connection of its own to Home Assistant's local address.
struct Local {
    live: Arc<AtomicBool>,
    /// Only the newest frame waits: a slow connection skips frames instead
    /// of piling them up in memory.
    out: watch::Sender<Option<Value>>,
}

#[derive(Default)]
pub(super) struct Screens {
    grant: Option<Grant>,
    /// The Mini Monitor picture last sent for each pad.
    sent: HashMap<String, u64>,
    next_at: Option<Instant>,
    local: Option<Local>,
}

impl Screens {
    /// The integration accepted `connect`: frames may go, with this key.
    pub fn granted(&mut self, shared: &Shared, grant: Grant) {
        self.stop();
        let wanted = {
            let cfg = lock(&shared.cfg);
            local_address(&cfg, grant.local_url.as_deref())
        };
        self.local = wanted.map(|(url, token)| start_local(url, token));
        self.grant = Some(grant);
    }

    /// The integration isn't listening any more.
    pub fn stop(&mut self) {
        self.grant = None;
        self.sent.clear();
        self.next_at = None;
        self.local = None;
    }

    /// Whether frames go over the local address now.
    pub fn local_live(&self) -> bool {
        self.local.as_ref().is_some_and(|local| local.live.load(Ordering::SeqCst))
    }

    /// When frames might next be due: soon, while a watched pad shows Mini
    /// Monitor.
    pub fn next_due(&self, shared: &Shared, watched: &[String]) -> Option<Instant> {
        self.grant.as_ref()?;
        let cfg = lock(&shared.cfg);
        let showing = watched.iter().any(|pad_id| cfg.pad(pad_id).is_some_and(|entry| cfg.monitor_on(entry.kind)));
        showing.then(|| self.next_at.unwrap_or_else(Instant::now))
    }

    /// The frames due now. Those the local connection can take go there;
    /// the rest are returned, to go the usual way.
    pub fn due(&mut self, shared: &Shared, watched: &[String]) -> Vec<Value> {
        let Some(grant) = self.grant.clone() else { return Vec::new() };
        let now = Instant::now();
        if self.next_at.is_some_and(|at| now < at) {
            return Vec::new();
        }
        let (fps, seamless, wanted) = {
            let cfg = lock(&shared.cfg);
            let wanted: Vec<(String, PadKind, Vec<(String, usize, usize)>)> = watched
                .iter()
                .filter_map(|pad_id| {
                    let entry = cfg.pad(pad_id)?;
                    cfg.monitor_on(entry.kind).then(|| (pad_id.clone(), entry.kind, tile_names(&cfg, pad_id)))
                })
                .collect();
            (cfg.ha_monitor_fps.clamp(1, 15), cfg.on_screen_seamless, wanted)
        };
        let instance_id = lock(&shared.cfg).instance_id.clone();
        let mut main = Vec::new();
        for (pad_id, kind, names) in wanted {
            let Some(shown) = crate::monitor::shown(kind) else { continue };
            if self.sent.get(&pad_id) == Some(&shown.version) {
                continue;
            }
            let Some(message) = frame_message(&instance_id, &grant.key, &pad_id, &shown, &names, seamless) else { continue };
            self.sent.insert(pad_id, shown.version);
            match self.local.as_ref().filter(|local| local.live.load(Ordering::SeqCst)) {
                Some(local) => {
                    if let Err(refused) = local.out.send(Some(message)) {
                        main.extend(refused.0);
                    }
                }
                None => main.push(message),
            }
            self.next_at = Some(now + Duration::from_millis(1000 / fps as u64));
        }
        main
    }
}

/// The local address to send frames to, with the token, when the person
/// allows it and it isn't the address the app already uses.
fn local_address(cfg: &Config, local_url: Option<&str>) -> Option<(String, String)> {
    let local = websocket_address(local_url?);
    if !cfg.ha_monitor_local || local == websocket_address(&cfg.ha_url) {
        return None;
    }
    let token = crate::ha::unprotect(&cfg.ha_token)?;
    Some((local, token))
}

/// Each Mini Monitor tile's control: its column and row in the picture's
/// layout, and the name the card knows it by.
fn tile_names(cfg: &Config, pad_id: &str) -> Vec<(String, usize, usize)> {
    let (Some(entry), Some(index)) = (cfg.pad(pad_id), cfg.pad_profile_of(pad_id)) else { return Vec::new() };
    let profile = &cfg.profiles[index];
    let layout = layout_of(entry, profile);
    crate::monitor::layout_for(cfg, entry.kind)
        .tiles
        .iter()
        .filter_map(|tile| {
            let name = match entry.kind {
                PadKind::Main if tile.is_band() => format!("band_{}", tile.row + 1),
                PadKind::Main | PadKind::Dashboard => layout.name_of(tile.row * profile.cols + tile.column)?.to_string(),
                PadKind::Akp03e => layout.name_of(small_cell_of_key(tile.row * 3 + tile.column))?.to_string(),
            };
            Some((name, tile.column, tile.row))
        })
        .collect()
}

/// A `frame` message for one picture. `seamless` asks for one picture over
/// the keys and the gaps, as pads on screen show it.
fn frame_message(instance_id: &str, key: &str, pad_id: &str, shown: &Shown, names: &[(String, usize, usize)], seamless: bool) -> Option<Value> {
    let (width, height) = (usize::try_from(shown.layout.width).ok()?, usize::try_from(shown.layout.height).ok()?);
    let data = crate::monitor::rgb_jpeg(&shown.canvas, width, height, QUALITY, jpeg_encoder::SamplingFactor::F_2_2)?;
    let tiles: Vec<Value> = shown
        .layout
        .tiles
        .iter()
        .filter_map(|tile| {
            let (name, ..) = names.iter().find(|(_, column, row)| (*column, *row) == (tile.column, tile.row))?;
            Some(json!({ "name": name, "x": tile.x, "y": tile.y, "w": tile.width, "h": tile.height }))
        })
        .collect();
    let mut message = json!({
        "type": "stream_dock_ha/frame",
        "instance_id": instance_id,
        "key": key,
        "pad_id": pad_id,
        "width": width,
        "height": height,
        "data": super::pictures::base64(&data),
        "tiles": tiles,
    });
    // Only when on: integrations before 0.1.3 refuse a field they don't know.
    if seamless {
        message["seamless"] = json!(true);
    }
    Some(message)
}

/// Keeps a connection to the local address for as long as it's wanted,
/// trying again now and then. Frames sent while it's down are dropped by
/// `due`, which then sends them the usual way.
fn start_local(url: String, token: String) -> Local {
    let live = Arc::new(AtomicBool::new(false));
    let (out, frames) = watch::channel(None);
    tokio::spawn(run_local(url, token, live.clone(), frames));
    Local { live, out }
}

async fn run_local(url: String, token: String, live: Arc<AtomicBool>, mut frames: watch::Receiver<Option<Value>>) {
    loop {
        if let Ok(Ok((mut socket, _))) = tokio::time::timeout(LOCAL_TIMEOUT, tokio_tungstenite::connect_async(url.as_str())).await {
            if log_in(&mut socket, &token).await {
                crate::pad::note(&format!("Mini Monitor frames for Home Assistant go over {url}"));
                live.store(true, Ordering::SeqCst);
                let mut id = 0u64;
                loop {
                    tokio::select! {
                        changed = frames.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            let Some(mut message) = frames.borrow_and_update().clone() else { continue };
                            id += 1;
                            message["id"] = json!(id);
                            if socket.send(Message::text(message.to_string())).await.is_err() {
                                break;
                            }
                        }
                        // Answers are read and dropped; a closed socket ends it.
                        heard = socket.next() => match heard {
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            Some(Ok(_)) => {}
                        },
                    }
                }
                live.store(false, Ordering::SeqCst);
            }
        }
        // Waits, but stops at once when the frames are no longer wanted.
        let wait = tokio::time::sleep(LOCAL_RETRY);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                changed = frames.changed() => if changed.is_err() { return },
            }
        }
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn log_in(socket: &mut Socket, token: &str) -> bool {
    if read(socket).await.is_none_or(|hello| hello["type"] != "auth_required") {
        return false;
    }
    let auth = json!({ "type": "auth", "access_token": token });
    if socket.send(Message::text(auth.to_string())).await.is_err() {
        return false;
    }
    read(socket).await.is_some_and(|answer| answer["type"] == "auth_ok")
}

/// The next message, if one comes soon and it's JSON.
async fn read(socket: &mut Socket) -> Option<Value> {
    match tokio::time::timeout(LOCAL_TIMEOUT, socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grant_takes_the_key_and_a_real_local_address() {
        let grant = Grant::of(&json!({ "frame_key": "abc", "local_url": "http://192.168.1.20:8123" }));
        assert_eq!(grant, Grant { key: "abc".into(), local_url: Some("http://192.168.1.20:8123".into()) });
        assert_eq!(Grant::of(&json!({ "frame_key": "abc", "local_url": null })).local_url, None);
        assert_eq!(Grant::of(&json!({ "local_url": " " })), Grant::default());
    }

    #[test]
    fn the_local_address_is_used_only_when_it_differs_and_is_allowed() {
        let mut cfg = Config::sample();
        cfg.ha_url = "https://home.example.org".into();
        cfg.ha_token = crate::ha::protect("secret").unwrap();
        let local = local_address(&cfg, Some("http://192.168.1.20:8123"));
        assert_eq!(local, Some(("ws://192.168.1.20:8123/api/websocket".into(), "secret".into())));
        assert_eq!(local_address(&cfg, None), None);
        cfg.ha_url = "http://192.168.1.20:8123".into();
        assert_eq!(local_address(&cfg, Some("http://192.168.1.20:8123")), None, "already the address in use");
        cfg.ha_url = "https://home.example.org".into();
        cfg.ha_monitor_local = false;
        assert_eq!(local_address(&cfg, Some("http://192.168.1.20:8123")), None, "the person said no");
    }

    #[test]
    fn every_mini_monitor_tile_has_its_control() {
        let mut cfg = Config::sample();
        cfg.add_small_pad("Small");
        cfg.repair();
        let main = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        cfg.monitor_area = crate::model::MonitorArea::KeysAndBand;
        let names = tile_names(&cfg, &main);
        assert_eq!(names.len(), 18);
        assert!(names.contains(&("key_1".into(), 0, 0)) && names.contains(&("key_7".into(), 1, 1)) && names.contains(&("band_3".into(), 5, 2)));
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        let names = tile_names(&cfg, &small);
        assert_eq!(names.iter().map(|(name, ..)| name.as_str()).collect::<Vec<_>>(), ["key_1", "key_2", "key_3", "key_4", "key_5", "key_6"]);
    }

    #[test]
    fn a_frame_is_one_jpeg_with_each_tiles_place() {
        let layout = crate::monitor::Layout::of(crate::model::MonitorArea::KeysOnly, None);
        let canvas = layout.alignment_pattern();
        let shown = Shown { layout: layout.clone(), canvas: Arc::new(canvas), version: 3, from: None };
        let names = vec![("key_1".to_string(), 0, 0), ("key_2".to_string(), 1, 0)];
        let message = frame_message("1111111111111111", "key", "2222222222222222", &shown, &names, true).unwrap();
        assert_eq!(message["seamless"], true);
        let plain = frame_message("1111111111111111", "key", "2222222222222222", &shown, &names, false).unwrap();
        assert!(plain.get("seamless").is_none(), "left out, for older integrations");
        assert_eq!((message["width"].as_i64(), message["height"].as_i64()), (Some(layout.width as i64), Some(layout.height as i64)));
        assert_eq!(message["tiles"], json!([{ "name": "key_1", "x": 0, "y": 0, "w": 85, "h": 85 }, { "name": "key_2", "x": 85, "y": 0, "w": 85, "h": 85 }]));
        assert!(message["data"].as_str().unwrap().starts_with("/9j/"), "a JPEG");
    }
}
