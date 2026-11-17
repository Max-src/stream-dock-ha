//! Alerts from Home Assistant (docs/integration-protocol.md, section 6): one
//! picture over all of a pad's screen keys, drawn across the gaps the way a
//! stretched camera is, with its words kept out of those gaps. Any press on that pad takes it away and does nothing
//! else. Alerts are only kept in memory, so they're gone after a restart.

use crate::model::layout::{Control, ControlType, Layout};
use crate::model::{lock, KeyStyle, PictureFit, Shared};
use crate::render::{alert_picture, AlertContent, CameraPart, Look};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// A strong amber, for an alert that doesn't choose a colour.
pub const AMBER: [u8; 3] = [255, 170, 0];
/// The icon of an alert that doesn't choose one.
pub const BELL: &str = "mdi:bell-ring";
/// How big a key is in the alert's own picture. The pads and the integration
/// take their parts of it at their own sizes.
const UNIT: u32 = 120;

/// One alert on one pad.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub title: String,
    pub message: String,
    pub icon: String,
    pub color: [u8; 3],
    /// A camera whose live picture fills the alert.
    pub camera: Option<String>,
    /// When it goes away by itself, if it does.
    pub until: Option<Instant>,
    /// Tells one alert from the next, even with the same words.
    pub serial: u64,
}

/// The alert showing on each pad, by pad id.
pub type Alerts = HashMap<String, Alert>;

static SERIAL: AtomicU64 = AtomicU64::new(0);

fn hash_of(value: &impl std::hash::Hash) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(value)
}

/// The alert a `show_alert` command asks for, or what's wrong with it.
pub fn from_command(data: &Value) -> Result<Alert, String> {
    let text = |field: &str| -> Result<String, String> {
        match data.get(field) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(text)) => Ok(text.clone()),
            Some(_) => Err(format!("\"{field}\" should be text")),
        }
    };
    let color = match data.get("color") {
        None | Some(Value::Null) => AMBER,
        Some(value) => serde_json::from_value::<[u8; 3]>(value.clone())
            .map_err(|_| "\"color\" should be red, green and blue, like [255, 170, 0]".to_string())?,
    };
    let seconds = match data.get("seconds") {
        None | Some(Value::Null) => 0.0,
        Some(value) => value.as_f64().filter(|seconds| *seconds >= 0.0).ok_or("\"seconds\" should be a number, 0 or more")?,
    };
    let camera = Some(text("camera")?).filter(|camera| !camera.is_empty());
    if camera.as_deref().is_some_and(|camera| !camera.starts_with("camera.")) {
        return Err("\"camera\" should be a camera entity, like camera.front_door".into());
    }
    let icon = text("icon")?;
    Ok(Alert {
        title: text("title")?,
        message: text("message")?,
        icon: if icon.is_empty() { BELL.into() } else { icon },
        color,
        camera,
        until: (seconds > 0.0).then(|| Instant::now() + Duration::from_secs_f64(seconds.min(86_400.0))),
        serial: 0,
    })
}

/// Puts an alert on a pad, in place of any it had. One with a time limit
/// takes itself away when that runs out.
pub fn show(shared: &Arc<Shared>, pad_id: &str, mut alert: Alert) {
    alert.serial = next_serial();
    let (serial, until) = (alert.serial, alert.until);
    lock(&shared.alerts).insert(pad_id.to_string(), alert);
    changed(shared);
    if let Some(until) = until {
        take_away_at(Timed { shared: Arc::downgrade(shared), name: pad_id.to_string(), serial, until, remove: remove_alert });
    }
}

/// A new serial number, telling one alert or flash from the next.
pub(crate) fn next_serial() -> u64 {
    SERIAL.fetch_add(1, Ordering::SeqCst) + 1
}

/// Something that goes away by itself, and when: an alert, or a picture on
/// one key (`flashes`). `name` tells what it is, and a new one under the same
/// name takes the place of the one before. `remove` takes it away unless
/// another with a different serial has taken its place.
pub(crate) struct Timed {
    pub shared: Weak<Shared>,
    pub name: String,
    pub serial: u64,
    pub until: Instant,
    pub remove: fn(&Shared, &str, u64),
}

/// The one thread that takes timed alerts away, for every pad, started with
/// the first of them.
static TIMER: Mutex<Option<Sender<Timed>>> = Mutex::new(None);
/// How many timer threads are running: one at most.
static TIMER_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Hands a timed alert or flash to the timer thread, starting it if need be.
pub(crate) fn take_away_at(timed: Timed) {
    let mut timer = lock(&TIMER);
    let sender = timer.get_or_insert_with(start_timer);
    if let Err(lost) = sender.send(timed) {
        // The thread can only have gone if it panicked: start another.
        let sender = timer.insert(start_timer());
        let _ = sender.send(lost.0);
    }
}

fn start_timer() -> Sender<Timed> {
    let (sender, receiver) = channel();
    TIMER_THREADS.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        run_timer(receiver);
        TIMER_THREADS.fetch_sub(1, Ordering::SeqCst);
    });
    sender
}

/// Sleeps until the earliest alert is due or a new one comes. A new alert on
/// a pad takes the place of the one before, so waiting alerts never pile up.
fn run_timer(receiver: Receiver<Timed>) {
    let mut waiting: Vec<Timed> = Vec::new();
    loop {
        let heard = match waiting.iter().map(|timed| timed.until).min() {
            Some(next) => receiver.recv_timeout(next.saturating_duration_since(Instant::now())),
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match heard {
            Ok(timed) => {
                waiting.retain(|old| old.shared.strong_count() > 0 && !(old.name == timed.name && old.shared.ptr_eq(&timed.shared)));
                waiting.push(timed);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let (due, later): (Vec<Timed>, Vec<Timed>) = waiting.into_iter().partition(|timed| timed.until <= now);
        waiting = later;
        due.into_iter().for_each(take_away);
    }
}

fn take_away(timed: Timed) {
    if let Some(shared) = timed.shared.upgrade() {
        (timed.remove)(&shared, &timed.name, timed.serial);
    }
}

/// Takes a timed alert away, unless another has taken its place.
fn remove_alert(shared: &Shared, pad_id: &str, serial: u64) {
    let mut alerts = lock(&shared.alerts);
    if alerts.get(pad_id).is_some_and(|alert| alert.serial == serial) {
        alerts.remove(pad_id);
        drop(alerts);
        changed(shared);
    }
}

/// Takes a pad's alert away, and says whether it had one.
pub fn clear(shared: &Shared, pad_id: &str) -> bool {
    let removed = lock(&shared.alerts).remove(pad_id).is_some();
    if removed {
        changed(shared);
    }
    removed
}

fn changed(shared: &Shared) {
    // A pad without an alert any more doesn't need its picture.
    let alerts = lock(&shared.alerts);
    let mut drawn = lock(&DRAWN);
    if let Some(drawn) = drawn.as_mut() {
        drawn.retain(|name, _| {
            let still = alerts.keys().any(|pad_id| picture_name(pad_id) == *name);
            if !still {
                crate::camera::forget(name);
            }
            still
        });
    }
    drop((drawn, alerts));
    crate::pad::wake();
    shared.touch();
}

/// The alert showing on a pad now, if any.
pub fn active(shared: &Shared, pad_id: &str) -> Option<Alert> {
    let alerts = lock(&shared.alerts);
    alerts.get(pad_id).filter(|alert| alert.until.is_none_or(|until| until > Instant::now())).cloned()
}

/// The cameras alerts are showing, so their pictures are fetched too.
pub fn cameras(shared: &Shared) -> Vec<String> {
    lock(&shared.alerts).values().filter_map(|alert| alert.camera.clone()).collect()
}

/// Where each screen key of a layout sits in the grid of keys, across then
/// down, by cell, and how many keys across and down they cover. Columns and
/// rows go by the keys' order, as the layout's gaps aren't the screen's.
fn grid(layout: &Layout) -> (Vec<(usize, [u8; 2])>, [u8; 2]) {
    let screens: Vec<_> = layout.controls.iter().filter(|control| control.kind == ControlType::Key && control.screen).collect();
    let place = |value: f32, of: fn(&Control) -> f32| {
        let mut lines: Vec<f32> = screens.iter().map(|control| of(control)).collect();
        lines.sort_by(f32::total_cmp);
        lines.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        lines.iter().position(|line| (line - value).abs() < 0.01).unwrap_or(0) as u8
    };
    let keys: Vec<(usize, [u8; 2])> =
        screens.iter().filter_map(|control| Some((control.cell?, [place(control.x, |c| c.x), place(control.y, |c| c.y)]))).collect();
    let span = keys.iter().fold([1, 1], |span: [u8; 2], (_, at)| [span[0].max(at[0] + 1), span[1].max(at[1] + 1)]);
    (keys, span)
}

/// The name an alert's picture is kept under, with the camera pictures.
fn picture_name(pad_id: &str) -> String {
    format!("alert:{pad_id}")
}

/// What each pad's alert picture was last drawn from, so it's drawn again
/// only when that changes.
static DRAWN: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Draws the alert's picture for the area its keys cover, unless it's
/// already drawn as it would be now.
fn draw(alert: &Alert, name: &str, span: [u8; 2], gaps: [u8; 2]) {
    let camera_version = alert.camera.as_deref().map_or(0, crate::camera::version);
    let wanted = hash_of(&(alert.serial, camera_version, span, gaps));
    if lock(&DRAWN).get_or_insert_with(HashMap::new).get(name) == Some(&wanted) {
        return;
    }
    let camera = alert.camera.as_deref().and_then(crate::camera::picture);
    let content = AlertContent {
        title: &alert.title,
        message: &alert.message,
        icon: &alert.icon,
        color: alert.color,
        camera: camera.as_deref(),
    };
    // Redrawn with each camera picture, or never again for a still alert.
    crate::camera::store_as(name, alert_picture(&content, span, gaps, UNIT), alert.camera.is_some());
    lock(&DRAWN).get_or_insert_with(HashMap::new).insert(name.to_string(), wanted);
}

/// Each screen key's part of a pad's alert, by cell, as `frames` shows them.
/// `outside` is the colour round the alert's shape.
pub fn looks(alert: &Alert, pad_id: &str, layout: &Layout, gaps: [u8; 2], style: &KeyStyle, outside: [u8; 3]) -> Vec<(usize, Look)> {
    let (keys, span) = grid(layout);
    let name = picture_name(pad_id);
    draw(alert, &name, span, gaps);
    let version = crate::camera::version(&name);
    keys.into_iter()
        .map(|(cell, at)| {
            let camera = CameraPart {
                entity: name.clone(),
                version,
                at,
                span,
                fit: PictureFit::Stretch,
                gaps,
                shape: style.shape,
                corner: style.corner,
                outside,
            };
            let look = Look {
                text: String::new(),
                glyph: String::new(),
                color: alert.color,
                image: None,
                image_changed: None,
                fit: PictureFit::Stretch,
                style: style.clone(),
                toggle: None,
                track: None,
                camera: Some(camera),
                graph: None,
                unavailable: false,
            };
            (cell, look)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, PadKind};
    use serde_json::json;

    #[test]
    fn a_command_becomes_an_alert_with_sensible_defaults() {
        let alert = from_command(&json!({"title": "Door"})).unwrap();
        assert_eq!((alert.title.as_str(), alert.color, alert.icon.as_str(), alert.until), ("Door", AMBER, BELL, None));
        let timed = from_command(&json!({"message": "Ring", "color": [10, 20, 30], "seconds": 5, "icon": "mdi:doorbell"})).unwrap();
        assert_eq!((timed.color, timed.icon.as_str()), ([10, 20, 30], "mdi:doorbell"));
        assert!(timed.until.is_some_and(|until| until > Instant::now() + Duration::from_secs(4)));
        assert!(from_command(&json!({"color": "red"})).is_err());
        assert!(from_command(&json!({"seconds": -1})).is_err());
        assert!(from_command(&json!({"camera": "light.desk"})).is_err());
        assert_eq!(from_command(&json!({"camera": "camera.door"})).unwrap().camera.as_deref(), Some("camera.door"));
    }

    #[test]
    fn an_alert_shows_until_cleared_and_a_new_one_replaces_it() {
        let (shared, _opened) = Shared::new(Config::default(), true);
        let pad = lock(&shared.cfg).pads[0].id.clone();
        assert!(active(&shared, &pad).is_none());
        show(&shared, &pad, from_command(&json!({"title": "One"})).unwrap());
        show(&shared, &pad, from_command(&json!({"title": "Two"})).unwrap());
        assert_eq!(active(&shared, &pad).unwrap().title, "Two");
        assert!(clear(&shared, &pad));
        assert!(!clear(&shared, &pad));
        assert!(active(&shared, &pad).is_none());
    }

    #[test]
    fn an_alert_with_seconds_goes_away_by_itself() {
        let (shared, _opened) = Shared::new(Config::default(), true);
        let pad = lock(&shared.cfg).pads[0].id.clone();
        show(&shared, &pad, from_command(&json!({"title": "Soon gone", "seconds": 0.2})).unwrap());
        assert!(active(&shared, &pad).is_some());
        std::thread::sleep(Duration::from_millis(350));
        assert!(active(&shared, &pad).is_none());
        assert!(lock(&shared.alerts).is_empty(), "taken away, not only hidden");
    }

    #[test]
    fn timed_alerts_share_one_thread_and_still_go_on_time() {
        let (shared, _opened) = Shared::new(Config::default(), true);
        let pad = lock(&shared.cfg).pads[0].id.clone();
        for n in 0..50 {
            show(&shared, &pad, from_command(&json!({"title": format!("Alert {n}"), "seconds": 3600})).unwrap());
        }
        assert_eq!(TIMER_THREADS.load(Ordering::SeqCst), 1, "one thread for every alert");
        // A short one in place of the long ones still goes, and the pads, the
        // card and the window hear of it.
        let mut changes = shared.changes.subscribe();
        show(&shared, &pad, from_command(&json!({"title": "Soon gone", "seconds": 0.2})).unwrap());
        changes.borrow_and_update();
        std::thread::sleep(Duration::from_millis(400));
        assert!(lock(&shared.alerts).is_empty());
        assert!(changes.has_changed().unwrap(), "everything showing it was told");
        assert_eq!(TIMER_THREADS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_alert_covers_the_screen_keys_as_one_stretched_picture() {
        let mut cfg = Config::default();
        cfg.add_small_pad("Small");
        let main = cfg.pads[0].clone();
        let layout = crate::model::layout::layout_of(&main, cfg.main_profile());
        let alert = from_command(&json!({"title": "Door", "message": "Someone's there"})).unwrap();
        let looks = looks(&alert, &main.id, &layout, main.gaps, &cfg.style, [0, 0, 0]);
        let profile = cfg.main_profile();
        assert_eq!(looks.len(), profile.rows * profile.cols);
        let (cell, last) = looks.last().unwrap();
        let camera = last.camera.as_ref().unwrap();
        assert_eq!((*cell, camera.at, camera.span, camera.gaps), (14, [4, 2], [5, 3], main.gaps));
        assert!(last.is_stretched());
        // The AKP03E's six keys are three across and two down.
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().clone();
        let layout = crate::model::layout::layout_of(&small, &cfg.profiles[1]);
        let looks = super::looks(&alert, &small.id, &layout, small.gaps, &cfg.style, [0, 0, 0]);
        assert_eq!(looks.len(), 6);
        assert_eq!(looks[5].1.camera.as_ref().unwrap().span, [3, 2]);
        // The picture is the alert's colour, and drawn once for both pads' parts.
        let part = crate::render::key_picture(&looks[0].1);
        assert_eq!(part.dimensions(), (85, 85));
        assert_eq!(part.get_pixel(42, 2).0, AMBER);
    }
    /// The alert on a 293S, an AKP03E and a three-wide virtual pad, as the
    /// card's pictures show them. With `STREAM_DOCK_HA_SHOT_ALERTS` naming a
    /// folder, the pictures are saved there to look at.
    #[test]
    fn an_alert_reads_on_every_shape_of_pad() {
        let mut cfg = Config::default();
        cfg.add_small_pad("Small");
        let wall = cfg.add_dashboard_pad("Wall", 3, 3);
        cfg.repair();
        let (shared, _opened) = Shared::new(cfg.clone(), true);
        let small = cfg.pad_of_kind(PadKind::Akp03e).unwrap().id.clone();
        let folder = std::env::var_os("STREAM_DOCK_HA_SHOT_ALERTS").map(std::path::PathBuf::from);
        let short = json!({"title": "Front door", "message": "Someone is at the front door", "icon": "mdi:doorbell"});
        let long = json!({"title": "Washing machine", "message": "The washing is done. Hang it out before it starts to rain this afternoon."});
        for (name, pad) in [("293s", cfg.pads[0].id.clone()), ("akp03e", small), ("dashboard-3-wide", wall)] {
            for (words, alert) in [("short", &short), ("long", &long)] {
                show(&shared, &pad, from_command(alert).unwrap());
                let frame = crate::frames::pad_frame(&shared, &cfg, &crate::ha::Entities::new(), &pad).unwrap();
                let picture = crate::frames::pad_snapshot(&cfg, &pad, &frame, 2);
                assert!(picture.width() > 0, "{name}");
                if let Some(folder) = &folder {
                    picture.save(folder.join(format!("alert-{name}-{words}.png"))).unwrap();
                }
            }
        }
    }
}
