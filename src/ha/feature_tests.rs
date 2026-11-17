//! History graphs and full templates against the stand-in Home Assistant,
//! and, when asked, against a real one.

use super::tests::{heard, keep_connecting, runtime, stand_in, until_connected, Act};
use super::*;
use crate::model::HistorySpan;

fn answer(act: &UnboundedSender<Act>, asked: &Value, result: Value) {
    act.send(Act::Send(json!({"id": asked["id"], "type": "result", "success": true, "result": result}))).unwrap();
}

#[test]
fn an_hour_comes_from_the_history_of_every_change() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        keep_connecting(&shared, &url);
        until_connected(&shared).await;
        let worker = shared.clone();
        let fetching = tokio::spawn(async move { history::fetch(&worker.ha, "sensor.hour", HistorySpan::Hour).await });
        let (_, asked) = heard(&mut heard_rx, "history/history_during_period").await;
        assert_eq!(asked["entity_ids"], json!(["sensor.hour"]));
        assert_eq!(asked["minimal_response"], true);
        let start = history::parse_time(asked["start_time"].as_str().unwrap()).unwrap();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        assert!((now - 3600.0 - start).abs() < 5.0, "it starts an hour ago");
        answer(&act, &asked, json!({"sensor.hour": [{"s": "20.5", "lu": start}, {"s": "21", "lu": start + 600.0}]}));
        let samples = fetching.await.unwrap().unwrap();
        assert_eq!(samples, vec![(start, Some(20.5)), (start + 600.0, Some(21.0))]);
    });
}

#[test]
fn a_day_comes_from_the_averages_or_else_the_history() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        keep_connecting(&shared, &url);
        until_connected(&shared).await;

        // With averages, they're all that's asked for.
        let worker = shared.clone();
        let fetching = tokio::spawn(async move { history::fetch(&worker.ha, "sensor.kept", HistorySpan::Day).await });
        let (_, asked) = heard(&mut heard_rx, "recorder/statistics_during_period").await;
        assert_eq!(asked["statistic_ids"], json!(["sensor.kept"]));
        answer(&act, &asked, json!({"sensor.kept": [{"start": 1_000_000.0, "end": 1_300_000.0, "mean": 18.5}]}));
        assert_eq!(fetching.await.unwrap().unwrap(), vec![(1000.0, Some(18.5))]);

        // Without, the history of every change is fetched instead.
        let worker = shared.clone();
        let fetching = tokio::spawn(async move { history::fetch(&worker.ha, "input_number.none", HistorySpan::SixHours).await });
        let (_, asked) = heard(&mut heard_rx, "recorder/statistics_during_period").await;
        answer(&act, &asked, json!({}));
        let (_, asked) = heard(&mut heard_rx, "history/history_during_period").await;
        assert_eq!(asked["entity_ids"], json!(["input_number.none"]));
        answer(&act, &asked, json!({"input_number.none": [{"s": "4", "lu": 2000.0}]}));
        assert_eq!(fetching.await.unwrap().unwrap(), vec![(2000.0, Some(4.0))]);

        // A refusal is passed on, for the graph to try again later.
        let worker = shared.clone();
        let fetching = tokio::spawn(async move { history::fetch(&worker.ha, "sensor.refused", HistorySpan::Hour).await });
        let (_, asked) = heard(&mut heard_rx, "history/history_during_period").await;
        act.send(Act::Send(json!({"id": asked["id"], "type": "result", "success": false,
            "error": {"code": "unknown_command", "message": "Unknown command."}}))).unwrap();
        assert!(matches!(fetching.await.unwrap(), Err(HaError::Refused { .. })));
    });
}

/// The wanted subscriptions for these titles, as the settings would give them.
fn titles(list: &[(&str, &str)]) -> BTreeMap<templates::Key, Value> {
    list.iter().map(|(template, entity)| ((template.to_string(), entity.to_string()), templates::message(template, entity, None))).collect()
}

#[test]
fn full_templates_show_home_assistants_rendering_until_it_fails() {
    runtime().block_on(async {
        let (url, mut heard_rx, act) = stand_in().await;
        let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
        keep_connecting(&shared, &url);
        until_connected(&shared).await;
        let (first, second) = ("{{ state | float | round(1) }} °C", "{{ broken(");
        let mut followed = templates::Followed::default();
        followed.update(&shared, &titles(&[(first, "sensor.feature_t"), (second, "sensor.feature_t")]));
        let mut asked = BTreeMap::new();
        while asked.len() < 2 {
            let (_, message) = heard(&mut heard_rx, "render_template").await;
            assert_eq!(message["variables"], json!({"entity_id": "sensor.feature_t"}));
            assert_eq!(message["report_errors"], true);
            let template = message["template"].as_str().unwrap().to_string();
            asked.insert(if template.ends_with(first) { first } else { second }, message);
        }
        let local = title(first, "sensor.feature_t", None);
        assert_eq!(local, "… °C", "the app's own filling until Home Assistant answers");

        answer(&act, &asked[first], Value::Null);
        act.send(Act::Send(json!({"id": asked[first]["id"], "type": "event", "event": {"result": "21.5 °C", "listeners": {}}}))).unwrap();
        // Home Assistant refuses a template it can't read.
        act.send(Act::Send(json!({"id": asked[second]["id"], "type": "result", "success": false,
            "error": {"code": "template_error", "message": "TemplateSyntaxError: unexpected end of template"}}))).unwrap();
        for _ in 0..200 {
            if templates::rendered(first, "sensor.feature_t").is_some() && templates::was_reported("broken") > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(title(first, "sensor.feature_t", None), "21.5 °C");
        assert_eq!(templates::was_reported("broken"), 1);
        assert_eq!(templates::rendered(second, "sensor.feature_t"), None);

        // A number comes back as text, and an error while rendering brings
        // back the app's own filling, noted once however often it happens.
        act.send(Act::Send(json!({"id": asked[first]["id"], "type": "event", "event": {"result": 22, "listeners": {}}}))).unwrap();
        for _ in 0..200 {
            if templates::rendered(first, "sensor.feature_t").as_deref() == Some("22") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(templates::rendered(first, "sensor.feature_t").as_deref(), Some("22"));
        for _ in 0..2 {
            let error = json!({"error": "ValueError: could not convert 'unknown' to float (feature test)", "level": "ERROR"});
            act.send(Act::Send(json!({"id": asked[first]["id"], "type": "event", "event": error}))).unwrap();
        }
        for _ in 0..200 {
            if templates::rendered(first, "sensor.feature_t").is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(title(first, "sensor.feature_t", None), local);
        assert_eq!(templates::was_reported("(feature test)"), 1);

        // A title taken out of the settings stops its subscription.
        followed.update(&shared, &titles(&[(second, "sensor.feature_t")]));
        let (_, stop) = heard(&mut heard_rx, "unsubscribe_events").await;
        assert_eq!(stop["subscription"], asked[first]["id"]);
        assert_eq!(followed.keys().len(), 1);
    });
}

/// The settings for the real check: a throwaway Home Assistant's address
/// and token, from `HA_E2E_URL` and `HA_E2E_TOKEN`. It only reads.
fn real_server() -> (String, String) {
    (websocket_address(&std::env::var("HA_E2E_URL").unwrap()), std::env::var("HA_E2E_TOKEN").unwrap().trim().to_string())
}

/// Renders a full template and fetches an hour and a day of history from a
/// real Home Assistant, changing nothing there:
/// `HA_E2E_URL=http://127.0.0.1:8124 HA_E2E_TOKEN=... HA_E2E_ENTITY=input_number.x cargo test real_server_renders -- --ignored --nocapture`
#[test]
#[ignore = "needs a real Home Assistant"]
fn real_server_renders_templates_and_gives_history() {
    let (url, token) = real_server();
    let entity = std::env::var("HA_E2E_ENTITY").unwrap();
    std::env::set_var("STREAM_DOCK_HA_CONFIG", std::env::temp_dir().join("stream-dock-ha-ha-real-test.json"));
    let (shared, _opened) = Shared::new(crate::model::Config::default(), true);
    runtime().block_on(async {
        let worker = shared.clone();
        tokio::spawn(async move { session(&worker, &url, &token).await });
        tokio::time::timeout(Duration::from_secs(15), until_connected(&shared)).await.expect("no login");
        while lock(&shared.ha.entities).is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let state = lock(&shared.ha.entities).get(&entity).map(|found| found.state.clone()).unwrap();
        let template = "{{ state | float | round(1) }} {{ unit }} ({{ friendly_name }}, {{ states('sun.sun') }}) {% if true %}yes{% endif %}";
        let wanted = {
            let entities = lock(&shared.ha.entities);
            BTreeMap::from([((template.to_string(), entity.clone()), templates::message(template, &entity, entities.get(&entity)))])
        };
        let mut followed = templates::Followed::default();
        followed.update(&shared, &wanted);
        let mut rendered = None;
        for _ in 0..100 {
            rendered = templates::rendered(template, &entity);
            if rendered.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        println!("state {state}, rendered: {rendered:?}");
        let rendered = rendered.expect("Home Assistant rendered nothing");
        assert!(rendered.ends_with("yes"), "{rendered}");
        let broken = "{{ states('sensor.x') | no_such_filter }}";
        followed.update(&shared, &BTreeMap::from([((broken.to_string(), entity.clone()), templates::message(broken, &entity, None))]));
        for _ in 0..100 {
            if templates::was_reported("no_such_filter") > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        println!("a broken template was reported: {}", templates::was_reported("no_such_filter"));
        assert_eq!(templates::rendered(broken, &entity), None);
        for span in [HistorySpan::Hour, HistorySpan::Day] {
            let samples = history::fetch(&shared.ha, &entity, span).await.unwrap();
            println!("{span:?}: {} samples, last {:?}", samples.len(), samples.last());
            assert!(!samples.is_empty());
            let points = history::resample(&samples, history::parse_time(&history::iso_time(std::time::SystemTime::now())).unwrap() + 60.0, span.length().as_secs_f64());
            assert!(points.last().unwrap().is_some());
        }
    });
}
