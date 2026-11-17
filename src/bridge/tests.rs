//! The bridge against a stand-in Home Assistant with the integration's
//! commands, as docs/integration-protocol.md describes them.

use super::*;
use crate::ha::tests::{heard, keep_connecting, runtime, stand_in_answering, Act, Heard};
use crate::model::{Action, DockKey, PadKind, RemoteAccess};
use std::collections::VecDeque;

/// What the stand-in integration answers to `connect`, one after another;
/// once they run out, it accepts.
type Answers = Arc<Mutex<VecDeque<Value>>>;

fn ok(id: &Value, result: Value) -> Value {
    json!({ "id": id, "type": "result", "success": true, "result": result })
}

fn refused(code: &str, message: &str) -> Value {
    json!({ "success": false, "error": { "code": code, "message": message } })
}

/// Answers the app's integration commands as the integration does.
fn integration(connects: Answers, states: Answers) -> impl Fn(&Value) -> Option<Value> + Send + 'static {
    move |message| {
        let id = &message["id"];
        let kind = message["type"].as_str().unwrap_or("");
        let queued = match kind {
            "stream_dock_ha/connect" => lock(&connects).pop_front(),
            "stream_dock_ha/state" => lock(&states).pop_front(),
            _ => None,
        };
        if let Some(mut answer) = queued {
            answer["id"] = id.clone();
            answer["type"] = json!("result");
            return Some(answer);
        }
        match kind {
            "stream_dock_ha/connect" => Some(ok(id, json!({ "protocol": 1, "integration_version": "0.1.0" }))),
            "stream_dock_ha/state" => Some(ok(id, json!({ "renew_pads": [] }))),
            "stream_dock_ha/pressed" | "stream_dock_ha/pictures" | "stream_dock_ha/reply" | "stream_dock_ha/frame" => Some(ok(id, Value::Null)),
            _ => None,
        }
    }
}

/// Runs a test, which fails rather than waits for good if something never comes.
fn within(test: impl std::future::Future<Output = ()>) {
    runtime().block_on(async { tokio::time::timeout(Duration::from_secs(20), test).await.expect("the test took too long") });
}

struct Run {
    shared: Arc<Shared>,
    heard: Heard,
    act: UnboundedSender<Act>,
    connects: Answers,
    states: Answers,
}

/// The app with a bridge, connected to a stand-in with the integration.
async fn start(cfg: Config) -> Run {
    start_with(cfg, Vec::new(), Vec::new()).await
}

/// `start`, with the first answers to `connect` and `state` chosen.
async fn start_with(cfg: Config, connects: Vec<Value>, states: Vec<Value>) -> Run {
    std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-bridge-test.json"));
    let (connects, states): (Answers, Answers) = (Arc::new(Mutex::new(connects.into())), Arc::new(Mutex::new(states.into())));
    let (url, heard, act) = stand_in_answering(integration(connects.clone(), states.clone())).await;
    let (shared, _opened) = Shared::new(cfg, true);
    let (notes, back) = link(&shared);
    tokio::spawn(run(shared.clone(), notes, back));
    keep_connecting(&shared, &url);
    Run { shared, heard, act, connects, states }
}

fn with_dashboard() -> (Config, String) {
    let mut cfg = Config::sample();
    cfg.ha_control = true;
    cfg.pc_name = "Test PC".into();
    // Loading gives every settings file an id; these were never loaded.
    cfg.renew_instance();
    let wall = cfg.add_dashboard_pad("Wall", 2, 3);
    let index = cfg.pad_profile_of(&wall).unwrap();
    cfg.profiles[index].pages[0].keys[0] = DockKey { title: "Lamp".into(), ..DockKey::default() };
    cfg.profiles[index].pages[0].keys[1] = DockKey { title: "Next".into(), actions: vec![Action::NextPage], ..DockKey::default() };
    cfg.profiles[index].add_page();
    cfg.profiles[index].active_page = 0;
    cfg.repair();
    (cfg, wall)
}

async fn until(what: impl Fn() -> bool) {
    for _ in 0..500 {
        if what() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("it never happened");
}

/// Nothing of this type comes for a while.
async fn nothing_of(heard: &mut Heard, kind: &str, wait: Duration) {
    let end = tokio::time::Instant::now() + wait;
    while let Ok(Some((_, message))) = tokio::time::timeout_at(end, heard.recv()).await {
        assert_ne!(message["type"], kind, "{message}");
    }
}

/// Sends an event down the `connect` subscription.
fn event(run: &Run, connect: &Value, event: Value) {
    run.act.send(Act::Send(json!({ "id": connect["id"], "type": "event", "event": event }))).unwrap();
}

#[test]
fn it_connects_then_sends_the_whole_state() {
    within(async {
        let (cfg, wall) = with_dashboard();
        let mut run = start(cfg).await;
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        let cfg = lock(&run.shared.cfg).clone();
        assert_eq!(connect["protocol"], 1);
        assert_eq!((connect["instance_id"].clone(), connect["name"].clone(), connect["control"].clone()), (json!(cfg.instance_id), json!("Test PC"), json!(true)));
        assert_eq!(connect["machine"], machine_hash());
        assert_eq!(connect["app_version"], env!("CARGO_PKG_VERSION"));
        let (_, state) = heard(&mut run.heard, "stream_dock_ha/state").await;
        assert_eq!(state["name"], "Test PC");
        let pads = state["pads"].as_array().unwrap();
        assert_eq!(pads.len(), 2);
        assert_eq!((pads[1]["id"].clone(), pads[1]["pages"].clone()), (json!(wall), json!(2)));
        assert!(pads[1]["layout"]["controls"].as_array().unwrap().iter().all(|control| control["type"] == "key"));
        until(|| run.shared.bridge.status() == Status::Connected).await;
        assert_eq!(run.shared.bridge.status().label("Test PC"), "Integration: connected as Test PC");
        assert!(run.shared.bridge.is_connected());
    });
}

#[test]
fn without_the_integration_the_app_carries_on_and_asks_at_the_next_login() {
    within(async {
        let mut run = start_with(with_dashboard().0, vec![refused("unknown_command", "Unknown command.")], Vec::new()).await;
        let (first, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        until(|| run.shared.bridge.status() == Status::NotInstalled).await;
        nothing_of(&mut run.heard, "stream_dock_ha/state", Duration::from_millis(400)).await;
        run.act.send(Act::Close).unwrap();
        let (second, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        assert_eq!((first, second), (0, 1));
        heard(&mut run.heard, "stream_dock_ha/state").await;
        until(|| run.shared.bridge.status() == Status::Connected).await;
    });
}

#[test]
fn while_home_assistant_starts_connect_is_asked_again_on_the_same_connection() {
    within(async {
        // Home Assistant answers before its integrations are loaded or set up.
        let refusals = vec![refused("unknown_command", "Unknown command."), refused("not_set_up", "Not set up yet")];
        let mut run = start_with(with_dashboard().0, refusals, Vec::new()).await;
        let (first, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        until(|| run.shared.bridge.status() == Status::NotInstalled).await;
        let (second, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        let (third, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        until(|| run.shared.bridge.status() == Status::Connected).await;
        // No new login was needed.
        assert_eq!((first, second, third), (0, 0, 0));
    });
}

#[test]
fn each_refusal_of_connect_shows_what_to_do() {
    within(async {
        let cases = [
            (refused("protocol_mismatch", "The integration speaks protocol 2"), Status::UpdateApp),
            (refused("protocol_mismatch", "The integration speaks protocol 0"), Status::UpdateIntegration),
            (refused("not_set_up", "Add the Stream Dock HA integration first"), Status::NotSetUp),
            (refused("unauthorized", "Only administrators"), Status::NeedsAdmin),
        ];
        let mut run = start(with_dashboard().0).await;
        heard(&mut run.heard, "stream_dock_ha/connect").await;
        until(|| run.shared.bridge.status() == Status::Connected).await;
        for (answer, status) in cases {
            lock(&run.connects).push_back(answer);
            run.act.send(Act::Close).unwrap();
            heard(&mut run.heard, "stream_dock_ha/connect").await;
            until(|| run.shared.bridge.status() == status).await;
            assert!(!run.shared.bridge.is_connected());
        }
        // An integration answering with another protocol is no better.
        lock(&run.connects).push_back(json!({ "success": true, "result": { "protocol": 3, "integration_version": "9" } }));
        run.act.send(Act::Close).unwrap();
        heard(&mut run.heard, "stream_dock_ha/connect").await;
        until(|| run.shared.bridge.status() == Status::UpdateApp).await;
        nothing_of(&mut run.heard, "stream_dock_ha/state", Duration::from_millis(400)).await;
    });
}

#[test]
fn settings_copied_from_another_pc_get_new_ids_and_connect_again() {
    within(async {
        let (cfg, wall) = with_dashboard();
        let mut run = start_with(cfg, vec![refused("renew_instance", "This instance id belongs to another PC")], Vec::new()).await;
        let (connection, first) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        let (again_on, again) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        assert_eq!(connection, again_on, "on the same socket");
        assert_ne!(first["instance_id"], again["instance_id"]);
        let cfg = lock(&run.shared.cfg).clone();
        assert_eq!(again["instance_id"], json!(cfg.instance_id));
        assert!(cfg.pad(&wall).is_none(), "the pads get new ids too");
        let (_, state) = heard(&mut run.heard, "stream_dock_ha/state").await;
        assert_eq!(state["pads"][0]["id"], json!(cfg.pads[0].id));
    });
}

#[test]
fn pads_known_from_another_pc_get_new_ids_and_the_state_goes_again() {
    within(async {
        let (cfg, wall) = with_dashboard();
        let mut run = start_with(cfg, Vec::new(), vec![json!({ "success": true, "result": { "renew_pads": [wall] } })]).await;
        let (_, first) = heard(&mut run.heard, "stream_dock_ha/state").await;
        assert_eq!(first["pads"][1]["id"], json!(wall));
        let (_, second) = heard(&mut run.heard, "stream_dock_ha/state").await;
        let fresh = second["pads"][1]["id"].as_str().unwrap().to_string();
        assert_ne!(fresh, wall);
        let cfg = lock(&run.shared.cfg).clone();
        let index = cfg.pad_profile_of(&fresh).expect("its profiles went with it");
        assert_eq!(cfg.profiles[index].pages[0].keys[0].title, "Lamp");
        assert_eq!(second["pads"][0]["id"], first["pads"][0]["id"], "the others stay");
    });
}

#[test]
fn not_connected_any_more_means_connect_again() {
    within(async {
        let mut run = start(with_dashboard().0).await;
        let (connection, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        until(|| run.shared.bridge.is_connected()).await;
        // The integration was reloaded: the next state is refused.
        lock(&run.states).push_back(refused("not_connected", "Send connect first"));
        lock(&run.shared.cfg).pc_name = "Renamed".into();
        run.shared.touch();
        heard(&mut run.heard, "stream_dock_ha/state").await;
        let (again_on, _) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        assert_eq!(connection, again_on);
        let (_, state) = heard(&mut run.heard, "stream_dock_ha/state").await;
        assert_eq!(state["name"], "Renamed");
    });
}

#[test]
fn changes_are_gathered_and_the_pcs_numbers_wait_their_turn() {
    within(async {
        let mut run = start(with_dashboard().0).await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        until(|| run.shared.bridge.is_connected()).await;
        // Five quick changes make one state, within the debounce time.
        let started = tokio::time::Instant::now();
        for title in ["a", "b", "c", "d", "e"] {
            let mut cfg = lock(&run.shared.cfg);
            cfg.profiles[0].pages[0].keys[0].title = title.into();
            drop(cfg);
            run.shared.touch();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        heard(&mut run.heard, "stream_dock_ha/state").await;
        assert!(started.elapsed() < Duration::from_millis(800));
        nothing_of(&mut run.heard, "stream_dock_ha/state", Duration::from_millis(400)).await;
        // The CPU alone waits for STATS_EVERY, counted from the last state.
        lock(&run.shared.stats).cpu = 77;
        run.shared.touch();
        let changed = tokio::time::Instant::now();
        let (_, state) = heard(&mut run.heard, "stream_dock_ha/state").await;
        assert_eq!(state["pc"]["cpu"], 77);
        assert!(changed.elapsed() >= Duration::from_millis(150), "it waited");
        // With nothing at all changing, it still comes every AT_LEAST.
        let quiet = tokio::time::Instant::now();
        heard(&mut run.heard, "stream_dock_ha/state").await;
        assert!(quiet.elapsed() >= AT_LEAST - Duration::from_millis(300));
    });
}

/// A bridge that has just sent the state as it is now, without a connection.
fn sent_just_now(shared: &Arc<Shared>) -> Bridge {
    let (back, _) = unbounded_channel();
    Bridge {
        shared: shared.clone(),
        back,
        connect: None,
        sent_as: None,
        changed_as: None,
        renewals: 0,
        live: true,
        sent_state: Some(state::state_message(shared)),
        sent_at: Instant::now(),
        dirty_since: None,
        stats_due: None,
        pictures: pictures::Watching::default(),
        screens: screens::Screens::default(),
        retry_at: None,
        retry_wait: Duration::ZERO,
    }
}

#[test]
fn the_pcs_numbers_going_back_leave_nothing_due() {
    let (shared, _opened) = Shared::new(with_dashboard().0, true);
    let mut bridge = sent_just_now(&shared);
    let was = lock(&shared.stats).cpu;
    // The CPU changes, so a state waits for STATS_EVERY...
    lock(&shared.stats).cpu = was.wrapping_add(40);
    bridge.dirty_since = Some(Instant::now() - DEBOUNCE);
    bridge.send_state_if_due();
    assert!(bridge.stats_due.is_some());
    // ...then goes back to what was sent before that time comes.
    lock(&shared.stats).cpu = was;
    bridge.dirty_since = Some(Instant::now() - DEBOUNCE);
    bridge.send_state_if_due();
    std::thread::sleep(STATS_EVERY + Duration::from_millis(50));
    bridge.send_state_if_due();
    let due = bridge.next_due().expect("AT_LEAST is always due some time");
    assert!(due > Instant::now(), "the bridge would wake again at once, over and over");
    assert_eq!(bridge.sent_at + AT_LEAST, due);
}

#[test]
fn a_command_is_carried_out_and_answered() {
    within(async {
        let (cfg, wall) = with_dashboard();
        let mut run = start(cfg).await;
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        let command = json!({ "command": "show_page", "pad": wall, "page": 2, "source": "action", "user": "Max" });
        event(&run, &connect, json!({ "kind": "command", "request_id": "r-0123456789abcdef", "data": command }));
        let (_, reply) = heard(&mut run.heard, "stream_dock_ha/reply").await;
        assert_eq!((reply["request_id"].clone(), reply["ok"].clone(), reply["result"]["page"].clone()), (json!("r-0123456789abcdef"), json!(true), json!(2)));
        let (_, state) = heard(&mut run.heard, "stream_dock_ha/state").await;
        assert_eq!(state["pads"][1]["page"], 2);
        // A refusal is an answer too.
        event(&run, &connect, json!({ "kind": "command", "request_id": "r-2", "data": { "command": "show_page", "pad": wall, "page": 9 } }));
        let (_, reply) = heard(&mut run.heard, "stream_dock_ha/reply").await;
        assert_eq!(reply["ok"], false);
        assert!(reply["error"].as_str().unwrap().contains("page 9"));
        // So is a snapshot, as a JPEG of the whole pad.
        event(&run, &connect, json!({ "kind": "command", "request_id": "r-3", "data": { "command": "snapshot", "pad": wall, "scale": 2 } }));
        let (_, reply) = heard(&mut run.heard, "stream_dock_ha/reply").await;
        assert_eq!(reply["result"]["content_type"], "image/jpeg");
        let picture = decode(reply["result"]["data"].as_str().unwrap());
        let cfg = lock(&run.shared.cfg).clone();
        let layout = layout_of(cfg.pad(&wall).unwrap(), &cfg.profiles[cfg.pad_profile_of(&wall).unwrap()]);
        assert_eq!(picture.width(), ((layout.width + 2.0 * crate::model::layout::EDGE) * 170.0).round() as u32);
    });
}

fn watch(pad_id: &str, scale: u32) -> Value {
    let mut pads = serde_json::Map::new();
    pads.insert(pad_id.to_string(), json!(scale));
    json!({ "kind": "watch", "pads": pads })
}

fn decode(text: &str) -> image::DynamicImage {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let values: Vec<u32> = text.bytes().filter(|byte| *byte != b'=').map(|byte| LETTERS.iter().position(|l| *l == byte).unwrap() as u32).collect();
    let mut bytes = Vec::new();
    for chunk in values.chunks(4) {
        let joined = chunk.iter().enumerate().fold(0u32, |all, (at, value)| all | (value << (18 - 6 * at)));
        bytes.extend((0..chunk.len() - 1).map(|at| (joined >> (16 - 8 * at)) as u8));
    }
    image::load_from_memory(&bytes).unwrap()
}

#[test]
fn a_watched_pad_with_mini_monitor_on_gets_whole_frames() {
    within(async {
        let mut cfg = with_dashboard().0;
        cfg.mini_monitor = true;
        cfg.monitor_pad = PadKind::Main;
        let main = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        let layout = crate::monitor::layout_for(&cfg, PadKind::Main);
        let mut run = start(cfg).await;
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        // Nothing goes until a card watches the pad.
        crate::monitor::publish(PadKind::Main, &layout, layout.alignment_pattern(), None);
        nothing_of(&mut run.heard, "stream_dock_ha/frame", Duration::from_millis(400)).await;
        event(&run, &connect, watch(&main, 2));
        until(|| run.shared.bridge.watched() == [main.clone()]).await;
        let (_, frame) = heard(&mut run.heard, "stream_dock_ha/frame").await;
        assert_eq!((frame["pad_id"].clone(), frame["width"].clone()), (json!(main), json!(layout.width)));
        assert_eq!(frame["tiles"].as_array().unwrap().len(), 15);
        assert_eq!(frame["tiles"][0]["name"], "key_1");
        // The same picture isn't sent twice; a new one is.
        nothing_of(&mut run.heard, "stream_dock_ha/frame", Duration::from_millis(400)).await;
        crate::monitor::publish(PadKind::Main, &layout, layout.alignment_pattern(), None);
        heard(&mut run.heard, "stream_dock_ha/frame").await;
        // Mini Monitor off: no more frames.
        lock(&run.shared.cfg).mini_monitor = false;
        crate::monitor::publish(PadKind::Main, &layout, layout.alignment_pattern(), None);
        nothing_of(&mut run.heard, "stream_dock_ha/frame", Duration::from_millis(400)).await;
    });
}

#[test]
fn frames_go_over_the_local_address_when_there_is_one() {
    within(async {
        // Home Assistant's local address: another stand-in, as it would be.
        let (local_url, mut local_heard, _local) = crate::ha::tests::stand_in().await;
        let port = local_url.trim_start_matches("ws://127.0.0.1:").trim_end_matches("/api/websocket").to_string();
        let mut cfg = with_dashboard().0;
        cfg.mini_monitor = true;
        cfg.monitor_pad = PadKind::Main;
        cfg.ha_url = "https://home.example.invalid".into();
        cfg.ha_token = crate::ha::protect("good").unwrap();
        let main = cfg.pad_of_kind(PadKind::Main).unwrap().id.clone();
        let layout = crate::monitor::layout_for(&cfg, PadKind::Main);
        let grant = json!({ "success": true, "result": {
            "protocol": 1, "integration_version": "0.1.2", "frame_key": "k1", "local_url": format!("http://127.0.0.1:{port}"),
        } });
        let mut run = start_with(cfg, vec![grant], Vec::new()).await;
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        // The app logs in there with its token.
        until(|| run.shared.bridge.frames_local()).await;
        event(&run, &connect, watch(&main, 2));
        crate::monitor::publish(PadKind::Main, &layout, layout.alignment_pattern(), None);
        let (_, frame) = heard(&mut local_heard, "stream_dock_ha/frame").await;
        assert_eq!((frame["key"].clone(), frame["pad_id"].clone()), (json!("k1"), json!(main)));
        assert!(frame["id"].is_number());
        nothing_of(&mut run.heard, "stream_dock_ha/frame", Duration::from_millis(400)).await;
    });
}

#[test]
fn a_watched_pad_gets_a_complete_set_then_only_what_changed() {
    within(async {
        let (cfg, wall) = with_dashboard();
        let mut run = start(cfg).await;
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        event(&run, &connect, watch(&wall, 2));
        let (_, first) = heard(&mut run.heard, "stream_dock_ha/pictures").await;
        assert_eq!((first["pad_id"].clone(), first["scale"].clone(), first["complete"].clone()), (json!(wall), json!(2), json!(true)));
        let pictures = first["pictures"].as_object().unwrap();
        assert_eq!(pictures.len(), 6);
        assert!(pictures["key_3"].is_null(), "an empty key is null");
        let lamp = decode(pictures["key_1"]["data"].as_str().unwrap());
        assert_eq!((lamp.width(), lamp.height()), (170, 170));
        // One key changes: only it comes.
        {
            let mut cfg = lock(&run.shared.cfg);
            let index = cfg.pad_profile_of(&wall).unwrap();
            cfg.profiles[index].pages[0].keys[0].title = "Lamp on".into();
        }
        run.shared.touch();
        let (_, next) = heard(&mut run.heard, "stream_dock_ha/pictures").await;
        assert_eq!(next["complete"], false);
        assert_eq!(next["pictures"].as_object().unwrap().keys().collect::<Vec<_>>(), ["key_1"]);
        assert_ne!(next["pictures"]["key_1"]["hash"], pictures["key_1"]["hash"]);
        // A dark screen is nothing on every key.
        crate::screen_off::set_screen(&wall, PadKind::Dashboard, false);
        run.shared.touch();
        let (_, dark) = heard(&mut run.heard, "stream_dock_ha/pictures").await;
        let dark = dark["pictures"].as_object().unwrap();
        assert!(dark.contains_key("key_1") && dark.values().all(Value::is_null));
        crate::screen_off::set_screen(&wall, PadKind::Dashboard, true);
        // A new scale is a new complete set; no watch, no pictures.
        event(&run, &connect, watch(&wall, 1));
        let (_, smaller) = heard(&mut run.heard, "stream_dock_ha/pictures").await;
        assert_eq!((smaller["complete"].clone(), smaller["scale"].clone()), (json!(true), json!(1)));
        let lamp = decode(smaller["pictures"]["key_1"]["data"].as_str().unwrap());
        assert_eq!(lamp.width(), 85);
        event(&run, &connect, json!({ "kind": "watch", "pads": {} }));
        tokio::time::sleep(Duration::from_millis(100)).await;
        run.shared.touch();
        nothing_of(&mut run.heard, "stream_dock_ha/pictures", Duration::from_millis(400)).await;
    });
}

#[test]
fn presses_are_told_with_their_source() {
    within(async {
        let (cfg, wall) = with_dashboard();
        let mut run = start(cfg).await;
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
        until(|| run.shared.bridge.is_connected()).await;
        crate::actions::press_pad(&run.shared, &wall, 0, PressSource::Screen);
        let (_, press) = heard(&mut run.heard, "stream_dock_ha/pressed").await;
        assert_eq!((press["pad_id"].clone(), press["control"].clone(), press["title"].clone()), (json!(wall), json!("key_1"), json!("Lamp")));
        assert_eq!((press["source"].clone(), press["dismissed"].clone(), press["page"].clone()), (json!("screen"), json!(false), json!(1)));
        assert!(lock(&run.shared.last_press).is_some());
        // The card presses through a command, and hears back.
        event(&run, &connect, json!({ "kind": "command", "request_id": "r-9", "data": { "command": "press_key", "pad": wall, "control": "key_1", "source": "dashboard" } }));
        // The answer goes first, then the press it made.
        let (_, reply) = heard(&mut run.heard, "stream_dock_ha/reply").await;
        assert_eq!((reply["ok"].clone(), reply["result"]["control"].clone()), (json!(true), json!("key_1")));
        let (_, press) = heard(&mut run.heard, "stream_dock_ha/pressed").await;
        assert_eq!(press["source"], "dashboard");
        // An alert takes the next press, which only dismisses it.
        crate::alerts::show(&run.shared, &wall, crate::alerts::from_command(&json!({ "title": "Door" })).unwrap());
        event(&run, &connect, json!({ "kind": "command", "request_id": "r-10", "data": { "command": "press_key", "pad": wall, "key": 1, "source": "dashboard" } }));
        let (_, reply) = heard(&mut run.heard, "stream_dock_ha/reply").await;
        assert_eq!(reply["result"]["dismissed"], true);
        let (_, press) = heard(&mut run.heard, "stream_dock_ha/pressed").await;
        assert_eq!(press["dismissed"], true);
        assert!(crate::alerts::active(&run.shared, &wall).is_none());
        // A key set to Never is refused, and nobody hears of a press.
        {
            let mut cfg = lock(&run.shared.cfg);
            let index = cfg.pad_profile_of(&wall).unwrap();
            cfg.profiles[index].pages[0].keys[0].remote = RemoteAccess::Deny;
        }
        event(&run, &connect, json!({ "kind": "command", "request_id": "r-11", "data": { "command": "press_key", "pad": wall, "key": 1, "source": "action" } }));
        let (_, reply) = heard(&mut run.heard, "stream_dock_ha/reply").await;
        assert_eq!(reply["ok"], false);
        nothing_of(&mut run.heard, "stream_dock_ha/pressed", Duration::from_millis(300)).await;
    });
}

#[test]
fn turned_off_it_never_asks() {
    within(async {
        let (mut cfg, _) = with_dashboard();
        cfg.ha_integration = false;
        let mut run = start(cfg).await;
        crate::ha::tests::until_connected(&run.shared).await;
        nothing_of(&mut run.heard, "stream_dock_ha/connect", Duration::from_millis(400)).await;
        assert_eq!(run.shared.bridge.status(), Status::Off);
        // Turned on, it connects at once.
        lock(&run.shared.cfg).ha_integration = true;
        run.shared.touch();
        heard(&mut run.heard, "stream_dock_ha/connect").await;
        heard(&mut run.heard, "stream_dock_ha/state").await;
    });
}

#[test]
fn a_new_name_or_control_setting_is_sent_with_connect_once_it_settles() {
    within(async {
        let mut run = start(with_dashboard().0).await;
        heard(&mut run.heard, "stream_dock_ha/connect").await;
        until(|| run.shared.bridge.status() == Status::Connected).await;
        // Typed a letter at a time, the name goes once, when it's done.
        for name in ["O", "Of", "Office"] {
            lock(&run.shared.cfg).pc_name = name.into();
            run.shared.touch();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        assert_eq!((connect["name"].clone(), connect["control"].clone()), (json!("Office"), json!(true)));
        // The old subscription ends after the new one is made, so the PC never seems to leave.
        let (_, stop) = heard(&mut run.heard, "unsubscribe_events").await;
        assert!(stop["subscription"].as_u64() < connect["id"].as_u64());
        until(|| run.shared.bridge.status() == Status::Connected).await;
        nothing_of(&mut run.heard, "stream_dock_ha/connect", Duration::from_millis(500)).await;
        lock(&run.shared.cfg).ha_control = false;
        run.shared.touch();
        let (_, connect) = heard(&mut run.heard, "stream_dock_ha/connect").await;
        assert_eq!((connect["name"].clone(), connect["control"].clone()), (json!("Office"), json!(false)));
    });
}
