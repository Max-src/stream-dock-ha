use super::*;

fn entity(state: &str) -> Entity {
    let (_, entity) = Entity::from_json(&json!({
        "entity_id": "sensor.desk",
        "state": state,
        "attributes": {"friendly_name": "Desk", "unit_of_measurement": "°C", "battery": 87},
    }))
    .unwrap();
    entity
}

#[test]
fn templates_fill_in_names_states_units_and_attributes() {
    let e = entity("28.63");
    assert_eq!(fill("{{friendly_name}}", "sensor.desk", Some(&e)), "Desk");
    assert_eq!(fill("{{ state }}{{unit}}", "sensor.desk", Some(&e)), "28.63°C");
    assert_eq!(fill("\nAC Fan\n{{state}}", "sensor.desk", Some(&e)), "\nAC Fan\n28.63");
    assert_eq!(fill("{{battery}}%", "sensor.desk", Some(&e)), "87%");
    assert_eq!(fill("{{friendly_name}}", "sensor.desk", None), "sensor.desk");
    assert_eq!(fill("plain {{unclosed", "x", None), "plain {{unclosed");
}

fn found(filter: &EntityFilter, id: &str, entity: &Entity, area: &str) -> bool {
    filter.matches(id, entity, area)
}

#[test]
fn search_finds_by_name_id_area_and_state_in_any_order() {
    let (id, mut e) = ("light.bureau_lampe", entity("on"));
    e.name = "Lampe du Bureau Café".into();
    let search = |words: &str| found(&EntityFilter { words: words.into(), ..Default::default() }, id, &e, "Séjour");
    assert!(search("lampe"));
    assert!(search("bureau_lampe"));
    assert!(search("bureau lampe"));
    assert!(search("cafe"));
    assert!(search("sejour"));
    assert!(search("LAMPE on"));
    assert!(!search("lampe kitchen"));
}

#[test]
fn filters_narrow_by_type_kind_area_and_availability() {
    let mut e = entity("21.5");
    e.device_class = "temperature".into();
    let id = "sensor.desk";
    let by = |filter: EntityFilter| found(&filter, id, &e, "Office");
    assert!(by(EntityFilter { domain: "sensor".into(), ..Default::default() }));
    assert!(!by(EntityFilter { domain: "light".into(), ..Default::default() }));
    assert!(by(EntityFilter { class: "temperature".into(), ..Default::default() }));
    assert!(!by(EntityFilter { class: "battery".into(), ..Default::default() }));
    assert!(by(EntityFilter { area: "Office".into(), ..Default::default() }));
    assert!(!by(EntityFilter { area: "Kitchen".into(), ..Default::default() }));
    let gone = entity("unavailable");
    let hide = EntityFilter { hide_unavailable: true, ..Default::default() };
    assert!(!found(&hide, id, &gone, ""));
    assert!(found(&EntityFilter::default(), id, &gone, ""));
}

#[test]
fn types_get_friendly_names() {
    assert_eq!(domain_label("binary_sensor"), "On/off sensors");
    assert_eq!(domain_label("water_heater"), "Water heater");
}

#[test]
fn areas_come_from_the_entity_or_else_its_device() {
    let areas = vec![json!({"area_id": "a", "name": "Office"}), json!({"area_id": "b", "name": "Kitchen"})];
    let devices = vec![json!({"id": "d", "area_id": "b"})];
    let entities = vec![
        json!({"entity_id": "light.one", "area_id": "a", "device_id": "d"}),
        json!({"entity_id": "light.two", "area_id": null, "device_id": "d"}),
        json!({"entity_id": "light.three", "area_id": null, "device_id": null}),
    ];
    let linked = link_areas(&areas, &devices, &entities);
    assert_eq!(linked.get("light.one").map(String::as_str), Some("Office"));
    assert_eq!(linked.get("light.two").map(String::as_str), Some("Kitchen"));
    assert_eq!(linked.get("light.three"), None);
}

#[test]
fn on_and_off_states_are_recognised() {
    assert_eq!(entity("on").is_on(), Some(true));
    assert_eq!(entity("off").is_on(), Some(false));
    assert_eq!(entity("28.63").is_on(), None);
}

#[test]
fn addresses_are_accepted_in_the_usual_forms() {
    let full = "wss://home.example.org/api/websocket";
    assert_eq!(websocket_address(full), full);
    assert_eq!(websocket_address("https://home.example.org/"), full);
    assert_eq!(websocket_address("home.example.org"), full);
    assert_eq!(websocket_address("http://10.0.0.2:8123"), "ws://10.0.0.2:8123/api/websocket");
    assert_eq!(rest_address("https://home.example.org/"), "https://home.example.org");
    assert_eq!(rest_address("http://10.0.0.2:8123"), "http://10.0.0.2:8123");
}

#[test]
fn service_calls_use_the_entitys_domain_unless_told_otherwise() {
    let call = call_message("input_select.ac_mode", "select_next", "").unwrap();
    assert_eq!(call["domain"], "input_select");
    assert_eq!(call["service"], "select_next");
    assert_eq!(call["target"]["entity_id"], "input_select.ac_mode");
    let call = call_message("light.desk", "homeassistant.toggle", "{\"a\":1}").unwrap();
    assert_eq!(call["domain"], "homeassistant");
    assert_eq!(call["service_data"]["a"], 1);
    assert!(call_message("light.desk", "toggle", "{nope").is_err());
}

#[test]
fn only_things_that_toggle_get_a_switch() {
    assert!(can_toggle("switch.desk") && can_toggle("input_boolean.ac") && can_toggle("light.lamp"));
    assert!(!can_toggle("binary_sensor.door") && !can_toggle("sensor.temperature") && !can_toggle("lock.front"));
}

#[test]
fn automations_can_be_triggered_and_sensors_offer_no_action() {
    assert_eq!(services_for("automation.night")[0], "trigger");
    assert_eq!(services_for("input_select.mode")[0], "select_next");
    assert!(services_for("sensor.temperature").is_empty());
}

#[test]
fn the_token_comes_back_only_through_protection() {
    let secret = "example-token-for-a-test";
    let stored = protect(secret).unwrap();
    assert!(!stored.contains(&to_hex(secret.as_bytes())));
    assert_eq!(unprotect(&stored).as_deref(), Some(secret));
    assert_eq!(unprotect(""), None);
    assert_eq!(unprotect("zz"), None);
}

/// Reads a server's public greeting over the real secure connection.
/// Nothing is sent, so no login happens: `HA_HELLO=address cargo test greeting -- --ignored --nocapture`
#[test]
#[ignore = "needs a real server"]
fn greeting_from_a_real_server() {
    let url = websocket_address(&std::env::var("HA_HELLO").unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (mut socket, _) = connect_async(url.as_str()).await.unwrap();
        let greeting = socket.next().await.unwrap().unwrap();
        let greeting: Value = serde_json::from_str(greeting.to_text().unwrap()).unwrap();
        println!("server says {} (Home Assistant {})", greeting["type"], greeting["ha_version"]);
        assert_eq!(greeting["type"], "auth_required");
    });
}

/// Checks the real server takes a call after keep-alives, using the app's
/// own connection settings. It targets an entity that doesn't exist, so
/// nothing in the house changes:
/// `HA_REAL=path-to-config.json cargo test real_server -- --ignored --nocapture`
#[test]
#[ignore = "needs a real server"]
fn real_server_takes_a_call_after_a_keep_alive() {
    let text = std::fs::read_to_string(std::env::var("HA_REAL").unwrap()).unwrap();
    let cfg: crate::model::Config = serde_json::from_str(&text).unwrap();
    let (url, token) = (websocket_address(&cfg.ha_url), unprotect(&cfg.ha_token).unwrap());
    std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-ha-real-test.json"));
    let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let worker = shared.clone();
        tokio::spawn(async move { session(&worker, &url, &token).await });
        while !matches!(*lock(&shared.ha.status), HaStatus::Connected(_)) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Keep-alives run at test speed here, so plenty go out in this time.
        tokio::time::sleep(Duration::from_secs(3)).await;
        println!("status before the call: {}", lock(&shared.ha.status).label());
        shared.ha.call("input_boolean.stream_dock_ha_no_such_entity", "toggle", "").unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        let refusal = lock(&shared.status).clone();
        println!("message after the call: '{refusal}'");
        assert!(!refusal.contains("Identifier"), "{refusal}");
    });
}

/// Counts what kinds of entity a real server has. It only reads states:
/// `HA_REAL=path-to-config.json cargo test ha::tests::real_server_entity_kinds -- --ignored --nocapture --exact`
#[test]
#[ignore = "needs a real server"]
fn real_server_gives_a_camera_picture() {
    let text = std::fs::read_to_string(std::env::var("HA_REAL").unwrap()).unwrap();
    let cfg: crate::model::Config = serde_json::from_str(&text).unwrap();
    let (base, token) = (rest_address(&cfg.ha_url), unprotect(&cfg.ha_token).unwrap());
    let states: Value = crate::camera::agent()
        .get(format!("{base}/api/states"))
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    let ids = states.as_array().unwrap().iter().filter_map(|state| state["entity_id"].as_str());
    let cameras: Vec<&str> = ids.filter(|id| id.starts_with("camera.")).collect();
    println!("cameras: {}", cameras.len());
    // Only sizes are printed: the pictures are of a real home.
    for camera in cameras {
        match crate::camera::fetch(&format!("{base}/api/camera_proxy/{camera}"), &token) {
            Ok(picture) => println!("a camera answered, size {:?}", picture.dimensions()),
            Err(_) => println!("a camera gave no picture"),
        }
    }
}

/// Fires a command at the real server and waits for it to come back. The
/// change is only made to a copy of the settings held in memory.
#[test]
#[ignore = "needs a real Home Assistant"]
fn real_server_delivers_a_command() {
    let text = std::fs::read_to_string(std::env::var("HA_REAL").unwrap()).unwrap();
    let mut cfg: crate::model::Config = serde_json::from_str(&text).unwrap();
    let (url, token) = (websocket_address(&cfg.ha_url), unprotect(&cfg.ha_token).unwrap());
    cfg.repair();
    cfg.ha_control = true;
    cfg.pads[0].brightness = 11;
    std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-ha-real-test.json"));
    let (shared, _opened) = Shared::new(cfg, true);
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let _standard = standard_subscriptions(&shared);
        let worker = shared.clone();
        tokio::spawn(async move { session(&worker, &url, &token).await });
        tokio::time::sleep(Duration::from_secs(3)).await;
        println!("connected: {}", matches!(*lock(&shared.ha.status), HaStatus::Connected(_)));
        shared.ha.send(json!({
            "type": "fire_event",
            "event_type": crate::remote::COMMAND_EVENT,
            "event_data": {"command": "set_brightness", "level": 37, "id": "stream-dock-ha-check"},
        }));
        for _ in 0..50 {
            if lock(&shared.cfg).brightness_of(crate::model::PadKind::Main) == 37 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        println!("command came back: {}", lock(&shared.cfg).brightness_of(crate::model::PadKind::Main) == 37);
        assert_eq!(lock(&shared.cfg).brightness_of(crate::model::PadKind::Main), 37);
    });
}

#[test]
#[ignore = "needs a real Home Assistant"]
fn real_server_entity_kinds() {
    let text = std::fs::read_to_string(std::env::var("HA_REAL").unwrap()).unwrap();
    let cfg: crate::model::Config = serde_json::from_str(&text).unwrap();
    let (url, token) = (websocket_address(&cfg.ha_url), unprotect(&cfg.ha_token).unwrap());
    std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-ha-real-test.json"));
    let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let worker = shared.clone();
        tokio::spawn(async move { session(&worker, &url, &token).await });
        while lock(&shared.ha.entities).is_empty() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let entities = lock(&shared.ha.entities);
    let mut kinds: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut icons: BTreeMap<String, usize> = BTreeMap::new();
    for (id, entity) in entities.iter() {
        *kinds.entry((domain_of(id).to_string(), entity.device_class.clone())).or_default() += 1;
        if let Some(icon) = entity.attributes.get("icon").and_then(Value::as_str) {
            *icons.entry(icon.to_string()).or_default() += 1;
        }
    }
    println!("{} entities", entities.len());
    for ((domain, class), count) in kinds {
        println!("{count:5}  {domain}  {class}");
    }
    println!("own icons: {icons:?}");
}

/// A stand-in Home Assistant, speaking the documented protocol.
/// The whole way round: Home Assistant fires a command, the key changes
/// and the answer comes back as an event.
#[test]
fn answers_a_command_from_home_assistant() {
    use tokio::net::TcpListener;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (answer_tx, mut answer_rx) = unbounded_channel::<Value>();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::text(json!({"type": "auth_required", "ha_version": "test"}).to_string())).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let message: Value = serde_json::from_str(&text).unwrap();
                match message["type"].as_str().unwrap_or("") {
                    "auth" => ws.send(Message::text(json!({"type": "auth_ok"}).to_string())).await.unwrap(),
                    "subscribe_events" if message["event_type"] == crate::remote::COMMAND_EVENT => {
                        let event = json!({"id": message["id"], "type": "event", "event": {
                            "event_type": crate::remote::COMMAND_EVENT,
                            "data": {"command": "set_key", "key": 2, "id": "abc", "set": {"title": "From HA"}},
                        }});
                        ws.send(Message::text(event.to_string())).await.unwrap();
                    }
                    "subscribe_events" if message["event_type"] == crate::remote::OLD_EVENTS[1].0 => {
                        let event = json!({"id": message["id"], "type": "event", "event": {
                            "event_type": crate::remote::OLD_EVENTS[1].0,
                            "data": {"command": "set_key", "key": 3, "id": "old", "set": {"title": "Old name"}},
                        }});
                        ws.send(Message::text(event.to_string())).await.unwrap();
                    }
                    "fire_event" => {
                        let _ = answer_tx.send(message);
                    }
                    _ => {}
                }
            }
        });

        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-control-test.json"));
        let cfg = crate::model::Config { ha_control: true, ..Default::default() };
        let (shared, _opened) = Shared::new(cfg, true);
        let _standard = standard_subscriptions(&shared);
        let worker = shared.clone();
        tokio::spawn(async move {
            session(&worker, &format!("ws://127.0.0.1:{port}"), "token").await;
        });
        let mut answers = Vec::new();
        for _ in 0..2 {
            answers.push(tokio::time::timeout(Duration::from_secs(5), answer_rx.recv()).await.unwrap().unwrap());
        }
        answers.sort_by_key(|answer| answer["event_data"]["id"].as_str().unwrap_or("").to_string());
        assert_eq!(answers[0]["event_type"], crate::remote::RESULT_EVENT);
        assert_eq!(answers[0]["event_data"]["ok"], true);
        assert_eq!(answers[0]["event_data"]["id"], "abc");
        // A command under the app's earlier name is answered under it too.
        assert_eq!(answers[1]["event_type"], crate::remote::OLD_EVENTS[1].1);
        assert_eq!(answers[1]["event_data"]["id"], "old");
        let cfg = lock(&shared.cfg);
        assert_eq!(cfg.profiles[0].pages[0].keys[1].title, "From HA");
        assert_eq!(cfg.profiles[0].pages[0].keys[2].title, "Old name");
    });
}

#[test]
fn talks_to_a_home_assistant_from_login_to_live_updates() {
    use tokio::net::TcpListener;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (called_tx, mut called_rx) = unbounded_channel::<Value>();
        let (go_tx, mut go_rx) = unbounded_channel::<()>();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let hello = json!({"type": "auth_required", "ha_version": "2026.10.1"});
            ws.send(Message::text(hello.to_string())).await.unwrap();
            let mut last_id = 0u64;
            // The number the app gave its state_changed subscription.
            let mut changes_id = 0u64;
            loop {
                tokio::select! {
                    _ = go_rx.recv() => {
                        let event = json!({"id": changes_id, "type": "event", "event": {
                            "event_type": "state_changed",
                            "data": {"entity_id": "switch.desk", "new_state":
                                {"entity_id": "switch.desk", "state": "on", "attributes": {"friendly_name": "Desk"}}},
                        }});
                        ws.send(Message::text(event.to_string())).await.unwrap();
                    }
                    message = ws.next() => {
                        let Some(Ok(Message::Text(text))) = message else { return };
                        let message: Value = serde_json::from_str(&text).unwrap();
                        // The real server refuses any number that doesn't go up.
                        if let Some(id) = message["id"].as_u64() {
                            if id <= last_id {
                                let refused = json!({"id": id, "type": "result", "success": false,
                                    "error": {"code": "id_reuse", "message": "Identifier values have to increase."}});
                                ws.send(Message::text(refused.to_string())).await.unwrap();
                                continue;
                            }
                            last_id = id;
                        }
                        if message["type"] == "subscribe_events" && message["event_type"] == "state_changed" {
                            changes_id = message["id"].as_u64().unwrap();
                        }
                        let reply = match message["type"].as_str().unwrap() {
                            "ping" => json!({"id": message["id"], "type": "pong"}),
                            "auth" if message["access_token"] == "good" => json!({"type": "auth_ok"}),
                            "auth" => json!({"type": "auth_invalid"}),
                            "get_states" => json!({"id": message["id"], "type": "result", "success": true, "result": [
                                {"entity_id": "switch.desk", "state": "off", "attributes": {"friendly_name": "Desk"}}]}),
                            "call_service" => { called_tx.send(message.clone()).unwrap(); json!({"id": message["id"], "type": "result", "success": true}) }
                            _ => json!({"id": message["id"], "type": "result", "success": true}),
                        };
                        ws.send(Message::text(reply.to_string())).await.unwrap();
                    }
                }
            }
        });

        std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-ha-test.json"));
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        let worker = shared.clone();
        let url = format!("ws://127.0.0.1:{port}/api/websocket");
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let _standard = standard_subscriptions(&shared);
        tokio::spawn(async move {
            let _ = ready_tx.send(());
            session(&worker, &url, "good").await
        });
        ready_rx.await.unwrap();

        let state = || lock(&shared.ha.entities).get("switch.desk").map(|e| e.state.clone());
        for _ in 0..100 {
            if state().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(state().as_deref(), Some("off"));
        assert_eq!(*lock(&shared.ha.status), HaStatus::Connected("2026.10.1".into()));

        // A press after a keep-alive is what failed on the real server.
        tokio::time::sleep(KEEP_ALIVE * 3).await;
        for _ in 0..2 {
            shared.ha.call("switch.desk", "toggle", "").unwrap();
            let called = tokio::time::timeout(Duration::from_secs(2), called_rx.recv()).await;
            let called = called.expect("the server refused the press").unwrap();
            assert_eq!(called["domain"], "switch");
            assert_eq!(called["service"], "toggle");
        }

        go_tx.send(()).unwrap();
        for _ in 0..100 {
            if state().as_deref() == Some("on") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(state().as_deref(), Some("on"));
    });
}

#[test]
fn saving_settings_and_a_new_entity_state_both_count_as_changes() {
    let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
    let mut changes = shared.changes.subscribe();
    let seen = *changes.borrow_and_update();
    shared.save_now();
    assert!(changes.has_changed().unwrap());
    let after_save = *changes.borrow_and_update();
    assert_ne!(after_save, seen);
    store_states(&shared, &[json!({"entity_id": "switch.desk", "state": "on", "attributes": {}})]);
    assert!(changes.has_changed().unwrap());
    assert_ne!(*changes.borrow_and_update(), after_save);
}

/// What a test tells the stand-in server to do on its current connection.
pub(crate) enum Act {
    Send(Value),
    Close,
}

pub(crate) type Heard = tokio::sync::mpsc::UnboundedReceiver<(usize, Value)>;

/// A stand-in Home Assistant that takes any number of connections, one after
/// another. It logs everyone in, answers the app's usual first requests and
/// pings, and passes on everything else it hears, with the connection's
/// number, so a test can answer as it likes.
pub(crate) async fn stand_in() -> (String, Heard, UnboundedSender<Act>) {
    stand_in_answering(|_| None).await
}

/// `stand_in`, also answering what `answer` has an answer for, such as the
/// integration's `stream_dock_ha/*` commands. Those are passed on as well,
/// so a test still hears them.
pub(crate) async fn stand_in_answering(answer: impl Fn(&Value) -> Option<Value> + Send + 'static) -> (String, Heard, UnboundedSender<Act>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://127.0.0.1:{}/api/websocket", listener.local_addr().unwrap().port());
    let (heard_tx, heard_rx) = unbounded_channel::<(usize, Value)>();
    let (act_tx, mut act_rx) = unbounded_channel::<Act>();
    tokio::spawn(async move {
        for connection in 0.. {
            let Ok((stream, _)) = listener.accept().await else { return };
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.send(Message::text(json!({"type": "auth_required", "ha_version": "test"}).to_string())).await.unwrap();
            loop {
                tokio::select! {
                    act = act_rx.recv() => match act {
                        Some(Act::Send(message)) => ws.send(Message::text(message.to_string())).await.unwrap(),
                        Some(Act::Close) | None => {
                            let _ = ws.close(None).await;
                            break;
                        }
                    },
                    message = ws.next() => {
                        let Some(Ok(Message::Text(text))) = message else { break };
                        let message: Value = serde_json::from_str(&text).unwrap();
                        let id = message["id"].clone();
                        let reply = match message["type"].as_str().unwrap_or("") {
                            "auth" => Some(json!({"type": "auth_ok"})),
                            "ping" => Some(json!({"id": id, "type": "pong"})),
                            "get_states" | "config/area_registry/list" | "config/device_registry/list" | "config/entity_registry/list" => {
                                Some(json!({"id": id, "type": "result", "success": true, "result": []}))
                            }
                            _ => None,
                        };
                        match reply {
                            Some(reply) => ws.send(Message::text(reply.to_string())).await.unwrap(),
                            None => {
                                let answered = answer(&message);
                                let _ = heard_tx.send((connection, message));
                                if let Some(answered) = answered {
                                    ws.send(Message::text(answered.to_string())).await.unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    (url, heard_rx, act_tx)
}

/// The next message of this type the stand-in heard.
pub(crate) async fn heard(heard: &mut Heard, kind: &str) -> (usize, Value) {
    loop {
        let (connection, message) = tokio::time::timeout(Duration::from_secs(5), heard.recv()).await.expect("nothing came").unwrap();
        if message["type"] == kind {
            return (connection, message);
        }
    }
}

pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
}

/// Keeps connecting, as the app does, until the test ends.
pub(crate) fn keep_connecting(shared: &Arc<Shared>, url: &str) {
    let (worker, url) = (shared.clone(), url.to_string());
    tokio::spawn(async move {
        loop {
            session(&worker, &url, "good").await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
}

pub(crate) async fn until_connected(shared: &Shared) {
    while !matches!(*lock(&shared.ha.status), HaStatus::Connected(_)) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn a_request_gets_its_own_answer_or_the_refusal() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        // Without a connection, the answer comes at once.
        assert_eq!(shared.ha.request(json!({"type": "test/echo"})).await.unwrap(), Err(HaError::NotConnected));
        keep_connecting(&shared, &url);
        until_connected(&shared).await;
        let first = shared.ha.request(json!({"type": "test/echo", "text": "one"}));
        let second = shared.ha.request(json!({"type": "test/refuse"}));
        let (_, echo) = heard(&mut heard_rx, "test/echo").await;
        let (_, refuse) = heard(&mut heard_rx, "test/refuse").await;
        assert!(refuse["id"].as_u64() > echo["id"].as_u64());
        // Answered the other way round, each still reaches its own asker.
        act.send(Act::Send(json!({"id": refuse["id"], "type": "result", "success": false,
            "error": {"code": "unknown_command", "message": "Unknown command."}}))).unwrap();
        act.send(Act::Send(json!({"id": echo["id"], "type": "result", "success": true, "result": {"text": echo["text"]}}))).unwrap();
        assert_eq!(first.await.unwrap(), Ok(json!({"text": "one"})));
        assert_eq!(
            second.await.unwrap(),
            Err(HaError::Refused { code: "unknown_command".into(), message: "Unknown command.".into() })
        );
    });
}

#[test]
fn a_subscription_hears_its_events_until_it_is_dropped() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        keep_connecting(&shared, &url);
        let (events_tx, mut events_rx) = unbounded_channel::<Value>();
        let subscription = shared.ha.subscribe(
            json!({"type": "subscribe_events", "event_type": "test_event"}),
            Box::new(move |event| {
                let _ = events_tx.send(event.clone());
            }),
        );
        let (_, asked) = heard(&mut heard_rx, "subscribe_events").await;
        assert_eq!(asked["event_type"], "test_event");
        act.send(Act::Send(json!({"id": asked["id"], "type": "result", "success": true, "result": null}))).unwrap();
        act.send(Act::Send(json!({"id": asked["id"], "type": "event", "event": {"data": {"n": 1}}}))).unwrap();
        // Something under another number is none of its business.
        act.send(Act::Send(json!({"id": 999, "type": "event", "event": {"data": {"n": 2}}}))).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), events_rx.recv()).await.unwrap().unwrap();
        assert_eq!(event["data"]["n"], 1);
        subscription.cancel();
        let (_, stop) = heard(&mut heard_rx, "unsubscribe_events").await;
        assert_eq!(stop["subscription"], asked["id"]);
        assert!(stop["id"].as_u64() > asked["id"].as_u64());
        // Events still on their way are left out.
        act.send(Act::Send(json!({"id": asked["id"], "type": "event", "event": {"data": {"n": 3}}}))).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(events_rx.try_recv().is_err());
    });
}

#[test]
fn a_closed_connection_fails_whatever_is_waiting() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        keep_connecting(&shared, &url);
        until_connected(&shared).await;
        let waiting = shared.ha.request(json!({"type": "test/never"}));
        heard(&mut heard_rx, "test/never").await;
        act.send(Act::Close).unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(5), waiting).await.expect("still waiting");
        assert_eq!(answer.unwrap(), Err(HaError::Dropped));
    });
}

#[test]
fn subscriptions_and_login_hooks_come_back_after_a_reconnection() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        let logins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = logins.clone();
        shared.ha.on_login(Box::new(move |shared| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // A hook can ask for things itself.
            let _ = shared.ha.request(json!({"type": "test/hello"}));
        }));
        let (events_tx, mut events_rx) = unbounded_channel::<u64>();
        let replies = Arc::new(Mutex::new(Vec::new()));
        let heard_replies = replies.clone();
        let _kept = shared.ha.subscribe_with(
            json!({"type": "subscribe_events", "event_type": "kept"}),
            Box::new(move |event| {
                let _ = events_tx.send(event["n"].as_u64().unwrap_or(0));
            }),
            Box::new(move |result| lock(&heard_replies).push(result.is_ok())),
        );
        let dropped = shared.ha.subscribe(json!({"type": "subscribe_events", "event_type": "dropped"}), Box::new(|_| {}));
        keep_connecting(&shared, &url);

        let mut asked = Vec::new();
        while asked.len() < 2 {
            asked.push(heard(&mut heard_rx, "subscribe_events").await.1);
        }
        let (first, hello) = heard(&mut heard_rx, "test/hello").await;
        let kept_id = asked.iter().find(|m| m["event_type"] == "kept").unwrap()["id"].clone();
        // The hook ran once the subscriptions were made again.
        assert!(hello["id"].as_u64() > kept_id.as_u64());
        act.send(Act::Send(json!({"id": kept_id, "type": "result", "success": false,
            "error": {"code": "unknown_command", "message": "Unknown command."}}))).unwrap();
        drop(dropped);
        heard(&mut heard_rx, "unsubscribe_events").await;
        act.send(Act::Close).unwrap();

        // The next connection makes only the subscription still wanted, even
        // though it was refused the first time, and runs the hook again.
        let (second, again) = heard(&mut heard_rx, "subscribe_events").await;
        assert_eq!((first, second), (0, 1));
        assert_eq!(again["event_type"], "kept");
        act.send(Act::Send(json!({"id": again["id"], "type": "result", "success": true, "result": null}))).unwrap();
        act.send(Act::Send(json!({"id": again["id"], "type": "event", "event": {"n": 7}}))).unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), events_rx.recv()).await.unwrap(), Some(7));
        heard(&mut heard_rx, "test/hello").await;
        assert_eq!(logins.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(*lock(&replies), [false, true]);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let late: Vec<Value> = std::iter::from_fn(|| heard_rx.try_recv().ok().map(|(_, m)| m)).collect();
        assert!(late.iter().all(|m| m["event_type"] != "dropped"), "{late:?}");
    });
}

#[test]
fn a_service_with_no_entity_runs_on_its_own() {
    let call = call_message("", "homeassistant.restart", "").unwrap();
    assert_eq!((call["domain"].as_str(), call["service"].as_str()), (Some("homeassistant"), Some("restart")));
    assert!(call.get("target").is_none(), "no target at all");
    assert!(call_message("", "restart", "").unwrap_err().contains("homeassistant.restart"));
}

#[test]
fn a_refusal_for_lack_of_rights_says_so() {
    assert!(dispatch::refusal_text("unauthorized", "Unauthorized").contains("isn't an administrator's"));
    assert_eq!(dispatch::refusal_text("not_found", "Service not found"), "Home Assistant: Service not found");
}

#[test]
fn keys_keep_their_last_states_while_home_assistant_stops_and_starts() {
    let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
    let desk = |state: &str| json!({"entity_id": "switch.desk", "state": state, "attributes": {"friendly_name": "Desk"}});
    let changed = |state: &str| json!({"data": {"entity_id": "switch.desk", "new_state": desk(state)}});
    let state = || lock(&shared.ha.entities).get("switch.desk").map(|e| e.state.clone());
    shared.ha.pretend_ready();
    store_states(&shared, &[desk("on")]);
    assert_eq!(state().as_deref(), Some("on"));

    // Stopping: entities go unavailable one by one, which keys don't take.
    shared.ha.running.store(false, Ordering::SeqCst);
    store_change(&shared, &changed("unavailable"));
    assert_eq!(state().as_deref(), Some("on"));
    // Starting: the states asked for at login only add what wasn't known.
    let lamp = json!({"entity_id": "light.lamp", "state": "off", "attributes": {}});
    store_states(&shared, &[desk("unavailable"), lamp]);
    assert_eq!(state().as_deref(), Some("on"));
    assert!(lock(&shared.ha.entities).contains_key("light.lamp"));
    assert_eq!(shared.ha.availability(false), Availability::Down { label: false, restarting: false });

    // Running again: everything as Home Assistant says.
    shared.ha.running.store(true, Ordering::SeqCst);
    store_states(&shared, &[desk("off")]);
    assert_eq!(state().as_deref(), Some("off"));
    assert!(!lock(&shared.ha.entities).contains_key("light.lamp"));
    assert_eq!(shared.ha.availability(true), Availability::Ready);
}

#[test]
fn the_restart_counts_as_restarting_until_home_assistant_is_back() {
    let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
    assert!(!shared.ha.restarting());
    *lock(&shared.ha.restarted_at) = Some(std::time::Instant::now());
    assert_eq!(shared.ha.availability(false), Availability::Down { label: false, restarting: true });
}
