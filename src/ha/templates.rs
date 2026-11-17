//! Titles written as full Home Assistant templates, such as
//! `{{ states('sensor.x') | round(1) }}`, which Home Assistant renders.
//!
//! A title whose every `{{ … }}` is a plain name, like `{{state}}`, is filled
//! in by the app itself, as before. Anything else is followed with a
//! `render_template` subscription, one for each template and entity however
//! many keys use it. Until Home Assistant answers, or if it can't (an account
//! that isn't an administrator, a mistake in the template), the app's own
//! filling is shown, so a key never goes blank.
//!
//! The key's entity reaches the template as `entity_id`, and a short preamble
//! sets `state`, `friendly_name`, `unit` and the attributes the template
//! names from `states[entity_id]`. Home Assistant then notices the entity
//! changing and renders again by itself. Passing the values themselves as
//! variables would freeze them at the moment of subscribing, so every change
//! of a sensor would mean subscribing again, with the old value shown
//! meanwhile.

use super::*;
use crate::model::{BandItem, Config};
use std::collections::BTreeSet;

/// A template and the entity it's rendered for.
pub(crate) type Key = (String, String);

/// What Home Assistant last rendered, for each template and entity.
static RESULTS: Mutex<BTreeMap<Key, String>> = Mutex::new(BTreeMap::new());
/// Problems already written to the log, so each is written once.
static REPORTED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Names a template can use that are set by the preamble, or that would hide
/// something Home Assistant's templates need if an attribute took them.
const NOT_ATTRIBUTES: &[&str] = &[
    "entity_id", "state", "friendly_name", "unit", "states", "state_attr", "is_state", "is_state_attr", "has_value",
    "now", "utcnow", "today_at", "as_timestamp", "float", "int", "bool", "iif", "area_name", "device_attr", "expand",
    "true", "false", "none", "and", "or", "not", "in", "is", "if", "else", "for", "set", "round", "max", "min",
];

fn is_name(text: &str) -> bool {
    let mut letters = text.chars();
    letters.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && letters.all(|letter| letter.is_ascii_alphanumeric() || letter == '_')
}

/// Whether the app can fill a template itself: no `{% %}` or `{# #}`, and
/// nothing but a plain name inside each `{{ }}`.
pub fn is_simple(template: &str) -> bool {
    if template.contains("{%") || template.contains("{#") {
        return false;
    }
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}") else { return true };
        if !is_name(rest[start + 2..start + end].trim()) {
            return false;
        }
        rest = &rest[start + end + 2..];
    }
    true
}

/// A title as it should show: Home Assistant's rendering of a full template
/// once there is one, otherwise the app's own filling.
pub fn title(template: &str, entity_id: &str, entity: Option<&Entity>) -> String {
    if !is_simple(template) {
        if let Some(rendered) = lock(&RESULTS).get(&(template.to_string(), entity_id.to_string())) {
            return rendered.clone();
        }
    }
    fill(template, entity_id, entity)
}

/// The words of a template that could be names.
fn words(template: &str) -> BTreeSet<&str> {
    template.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|word| is_name(word)).collect()
}

/// Sets what the app's own titles offer from the entity, so a full template
/// can use the same names. Only attributes the template mentions are set.
fn preamble(template: &str, entity: Option<&Entity>) -> String {
    let mut out = String::from(
        "{% set state = states(entity_id) %}\
         {% set friendly_name = state_attr(entity_id, 'friendly_name') or entity_id %}\
         {% set unit = state_attr(entity_id, 'unit_of_measurement') or '' %}",
    );
    if let Some(entity) = entity {
        let named = words(template);
        for name in entity.attributes.keys().filter(|name| named.contains(name.as_str()) && !NOT_ATTRIBUTES.contains(&name.as_str())) {
            out.push_str(&format!("{{% set {name} = state_attr(entity_id, '{name}') %}}"));
        }
    }
    out
}

/// The subscription that renders a template for an entity.
pub(crate) fn message(template: &str, entity_id: &str, entity: Option<&Entity>) -> Value {
    json!({
        "type": "render_template",
        "template": format!("{}{template}", preamble(template, entity)),
        "variables": { "entity_id": entity_id },
        "report_errors": true,
    })
}

/// Every full template in the settings, with the subscription for each.
pub(crate) fn wanted(cfg: &Config, entities: &Entities) -> BTreeMap<Key, Value> {
    let mut found = BTreeMap::new();
    let mut add = |template: &str, entity_id: &str| {
        if !template.trim().is_empty() && !is_simple(template) {
            let key = (template.to_string(), entity_id.to_string());
            found.entry(key).or_insert_with(|| message(template, entity_id, entities.get(entity_id)));
        }
    };
    let keys = cfg.profiles.iter().flat_map(|profile| &profile.pages).flat_map(|page| &page.keys);
    for binding in keys.filter_map(|key| key.ha_binding()) {
        for template in [&binding.title, &binding.on_title, &binding.off_title] {
            add(template, &binding.entity);
        }
    }
    for item in &cfg.band {
        if let BandItem::HomeAssistant { entity, label } = item {
            add(label, entity);
        }
    }
    found
}

/// What to stop and what to start to go from the subscriptions there are to
/// those wanted. One whose message changed, because the template now names
/// another attribute, is made again.
pub(crate) fn plan(current: &BTreeMap<Key, Value>, wanted: &BTreeMap<Key, Value>) -> (Vec<Key>, Vec<Key>) {
    let stop = current.iter().filter(|(key, message)| wanted.get(*key) != Some(message)).map(|(key, _)| key.clone());
    let start = wanted.iter().filter(|(key, message)| current.get(*key) != Some(message)).map(|(key, _)| key.clone());
    (stop.collect(), start.collect())
}

/// Writes a problem to the log the first time it happens.
fn report(key: &Key, problem: &str) {
    let line = format!("Home Assistant couldn't render the title '{}': {problem}", key.0);
    if lock(&REPORTED).insert(line.clone()) {
        #[cfg(not(test))]
        note(&line);
    }
}

/// What a subscription heard: a rendering, or a problem, after which the
/// app's own filling is shown again.
fn heard(shared: &Shared, key: &Key, event: &Value) {
    if let Some(error) = event.get("error") {
        report(key, error.as_str().unwrap_or("unknown error"));
        // A warning, such as a missing attribute, still renders.
        if event.get("level").and_then(Value::as_str) == Some("WARNING") {
            return;
        }
        lock(&RESULTS).remove(key);
    } else if let Some(result) = event.get("result") {
        let text = match result {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let mut results = lock(&RESULTS);
        if results.get(key) == Some(&text) {
            return;
        }
        results.insert(key.clone(), text);
    } else {
        return;
    }
    crate::pad::wake();
    shared.touch_soon();
}

/// The template subscriptions there are, kept in step with the settings.
#[derive(Default)]
pub(crate) struct Followed {
    made: BTreeMap<Key, (Value, Subscription)>,
}

impl Followed {
    pub(crate) fn keys(&self) -> BTreeMap<Key, Value> {
        self.made.iter().map(|(key, (message, _))| (key.clone(), message.clone())).collect()
    }

    /// Stops what's no longer wanted and starts what's new.
    pub(crate) fn update(&mut self, shared: &Arc<Shared>, wanted: &BTreeMap<Key, Value>) {
        let (stop, start) = plan(&self.keys(), wanted);
        for key in stop {
            self.made.remove(&key);
            if !wanted.contains_key(&key) {
                lock(&RESULTS).remove(&key);
            }
        }
        for key in start {
            let message = wanted[&key].clone();
            let (weak, for_event) = (Arc::downgrade(shared), key.clone());
            let (weak_reply, for_reply) = (Arc::downgrade(shared), key.clone());
            let subscription = shared.ha.subscribe_with(
                message.clone(),
                Box::new(move |event| {
                    if let Some(shared) = weak.upgrade() {
                        heard(&shared, &for_event, event);
                    }
                }),
                Box::new(move |reply| {
                    if let (Err(error), Some(shared)) = (reply, weak_reply.upgrade()) {
                        // Refused, perhaps because the account isn't an administrator.
                        report(&for_reply, &error.to_string());
                        if lock(&RESULTS).remove(&for_reply).is_some() {
                            shared.touch();
                        }
                    }
                }),
            );
            self.made.insert(key, (message, subscription));
        }
    }
}

async fn keep_in_step(shared: Arc<Shared>) {
    let mut followed = Followed::default();
    let mut changes = shared.changes.subscribe();
    loop {
        let wanted = {
            let cfg = lock(&shared.cfg);
            let entities = lock(&shared.ha.entities);
            wanted(&cfg, &entities)
        };
        followed.update(&shared, &wanted);
        if changes.changed().await.is_err() {
            return;
        }
        // Typing a title changes the settings with every letter.
        tokio::time::sleep(Duration::from_millis(700)).await;
        changes.borrow_and_update();
    }
}

/// Keeps the subscriptions in step with the titles in the settings, for as
/// long as the app runs.
pub fn start(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        if let Ok(runtime) = runtime {
            runtime.block_on(keep_in_step(shared));
        }
    });
}

#[cfg(test)]
pub(crate) fn rendered(template: &str, entity_id: &str) -> Option<String> {
    lock(&RESULTS).get(&(template.to_string(), entity_id.to_string())).cloned()
}

#[cfg(test)]
pub(crate) fn was_reported(part: &str) -> usize {
    lock(&REPORTED).iter().filter(|line| line.contains(part)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_are_simple_and_anything_else_is_full() {
        for simple in ["", "Desk", "{{friendly_name}}", "{{ state }}{{unit}}", "\nAC\n{{ battery_level }}", "{{unclosed"] {
            assert!(is_simple(simple), "{simple}");
        }
        for full in [
            "{{ states('sensor.x') | round(1) }}",
            "{{ state | float * 2 }}",
            "{% if is_state('light.a', 'on') %}On{% endif %}",
            "{{ now().hour }}",
            "{{ }}",
            "{# a note #}",
            "{{ a.b }}",
        ] {
            assert!(!is_simple(full), "{full}");
        }
    }

    fn sensor() -> Entity {
        let (_, entity) = Entity::from_json(&json!({"entity_id": "sensor.t", "state": "21.46",
            "attributes": {"friendly_name": "Desk", "unit_of_measurement": "°C", "battery": 80, "now": 1}}))
        .unwrap();
        entity
    }

    #[test]
    fn the_preamble_sets_the_usual_names_and_the_attributes_used() {
        let message = message("{{ state | float | round(1) }} {{ battery }} {{ now() }}", "sensor.t", Some(&sensor()));
        assert_eq!(message["type"], "render_template");
        assert_eq!(message["variables"], json!({"entity_id": "sensor.t"}));
        assert_eq!(message["report_errors"], true);
        let template = message["template"].as_str().unwrap();
        assert!(template.starts_with("{% set state = states(entity_id) %}"), "{template}");
        assert!(template.contains("{% set battery = state_attr(entity_id, 'battery') %}"));
        // An attribute named like one of Home Assistant's own would hide it.
        assert!(!template.contains("set now"));
        assert!(template.ends_with("{{ state | float | round(1) }} {{ battery }} {{ now() }}"));
        // Nothing but the template itself is shown: the preamble has no line breaks.
        assert!(!template.contains('\n'));
    }

    #[test]
    fn a_simple_title_is_filled_and_a_full_one_waits_for_home_assistant() {
        let entity = sensor();
        let full = "{{ state | float | round(1) }}°";
        assert_eq!(title("{{ state }}{{unit}}", "sensor.t", Some(&entity)), "21.46°C");
        assert_eq!(title(full, "sensor.title_test", Some(&entity)), "°");
        lock(&RESULTS).insert((full.to_string(), "sensor.title_test".to_string()), "21.5°".into());
        assert_eq!(title(full, "sensor.title_test", Some(&entity)), "21.5°");
        assert_eq!(title(full, "sensor.other", Some(&entity)), "°");
    }

    #[test]
    fn subscriptions_follow_the_titles_in_the_settings() {
        use crate::model::{Action, HaAction};
        let mut cfg = Config::sample();
        cfg.repair();
        let full = "{{ states('sensor.x') | round(1) }}";
        let binding = HaAction { entity: "sensor.t".into(), title: full.into(), on_title: "{{state}}".into(), ..Default::default() };
        cfg.profiles[0].pages[0].keys[0].actions = vec![Action::HomeAssistant(binding.clone())];
        cfg.profiles[0].pages[0].keys[1].actions = vec![Action::HomeAssistant(binding)];
        cfg.band[1] = BandItem::HomeAssistant { entity: "sensor.u".into(), label: "{{ state | int }}".into() };
        let entities = Entities::new();
        let first = wanted(&cfg, &entities);
        let mut keys: Vec<Key> = first.keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, [("{{ state | int }}".to_string(), "sensor.u".to_string()), (full.to_string(), "sensor.t".to_string())]);
        let (stop, start) = plan(&BTreeMap::new(), &first);
        assert_eq!((stop.len(), start.len()), (0, 2));
        assert_eq!(plan(&first, &first), (vec![], vec![]));

        // A template that changes is one stopped and one started.
        cfg.band[1] = BandItem::HomeAssistant { entity: "sensor.u".into(), label: "{{ state | int + 1 }}".into() };
        let second = wanted(&cfg, &entities);
        let (stop, start) = plan(&first, &second);
        assert_eq!(stop, [("{{ state | int }}".to_string(), "sensor.u".to_string())]);
        assert_eq!(start, [("{{ state | int + 1 }}".to_string(), "sensor.u".to_string())]);

        // A new attribute the template names means subscribing again.
        cfg.band[1] = BandItem::HomeAssistant { entity: "sensor.u".into(), label: "{{ battery | int }}".into() };
        let before = wanted(&cfg, &entities);
        let mut entities = Entities::new();
        entities.insert("sensor.u".into(), sensor());
        let after = wanted(&cfg, &entities);
        let (stop, start) = plan(&before, &after);
        assert_eq!((stop.len(), start.len()), (1, 1));
        assert_eq!(stop, start);
    }
}
