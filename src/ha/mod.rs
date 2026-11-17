//! Home Assistant, through its public WebSocket API: live entity states, and
//! calling services when a key is pressed.
//!
//! The access token is stored encrypted with Windows' own per-user protection
//! (DPAPI), so the settings file on its own can't reveal it.

use crate::model::{lock, Shared};
use crate::pad::note;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Notify;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

mod dispatch;
pub use dispatch::{HaError, Subscription};
pub mod history;
pub mod templates;
pub use templates::title;
use dispatch::{Dispatch, Outgoing, Reply, Wire};
use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};


#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entity {
    pub state: String,
    pub name: String,
    pub unit: String,
    pub device_class: String,
    pub attributes: Map<String, Value>,
}

impl Entity {
    fn from_json(state: &Value) -> Option<(String, Entity)> {
        let id = state.get("entity_id")?.as_str()?.to_string();
        let attributes = state.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default();
        let text = |key: &str| attributes.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let entity = Entity {
            state: state.get("state").and_then(Value::as_str).unwrap_or("").to_string(),
            name: text("friendly_name"),
            unit: text("unit_of_measurement"),
            device_class: text("device_class"),
            attributes,
        };
        Some((id, entity))
    }

    /// Whether this is an on or off kind of state, and which.
    pub fn is_on(&self) -> Option<bool> {
        match self.state.as_str() {
            "on" | "open" | "home" | "playing" | "unlocked" | "active" | "heat" | "cool" => Some(true),
            "off" | "closed" | "not_home" | "paused" | "idle" | "locked" | "standby" => Some(false),
            _ => None,
        }
    }
}

pub type Entities = BTreeMap<String, Entity>;
/// The area each entity is in, by name.
pub type Areas = BTreeMap<String, String>;

/// Lower case without accents, so "cafe" finds "Café".
fn folded(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// What the entity browser is narrowing down by. Empty means any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityFilter {
    pub words: String,
    pub domain: String,
    pub class: String,
    pub area: String,
    pub hide_unavailable: bool,
}

pub fn domain_of(entity_id: &str) -> &str {
    entity_id.split('.').next().unwrap_or("")
}

impl EntityFilter {
    /// Everything but the type, so each type can show how many it would find.
    #[cfg(test)]
    pub fn matches_words(&self, id: &str, entity: &Entity, area: &str) -> bool {
        self.matches_these_words(&self.search_words(), id, entity, area)
    }

    /// The search words, folded once for a whole list of entities.
    pub fn search_words(&self) -> Vec<String> {
        folded(&self.words).split_whitespace().map(str::to_string).collect()
    }

    /// As `matches_words`, with the words from `search_words`.
    pub fn matches_these_words(&self, words: &[String], id: &str, entity: &Entity, area: &str) -> bool {
        if self.hide_unavailable && matches!(entity.state.as_str(), "unavailable" | "unknown") {
            return false;
        }
        if !self.area.is_empty() && self.area != area {
            return false;
        }
        if !self.class.is_empty() && self.class != entity.device_class {
            return false;
        }
        if words.is_empty() {
            return true;
        }
        let haystack = folded(&format!("{} {id} {area} {} {}", entity.name, entity.state, entity.device_class));
        words.iter().all(|word| haystack.contains(word.as_str()))
    }

    #[cfg(test)]
    pub fn matches(&self, id: &str, entity: &Entity, area: &str) -> bool {
        (self.domain.is_empty() || self.domain == domain_of(id)) && self.matches_words(id, entity, area)
    }
}

/// A friendly name for a type of entity.
pub fn domain_label(domain: &str) -> String {
    let known = match domain {
        "light" => "Lights",
        "switch" => "Switches",
        "sensor" => "Sensors",
        "binary_sensor" => "On/off sensors",
        "automation" => "Automations",
        "script" => "Scripts",
        "scene" => "Scenes",
        "button" | "input_button" => "Buttons",
        "climate" => "Climate",
        "cover" => "Covers",
        "fan" => "Fans",
        "lock" => "Locks",
        "media_player" => "Media players",
        "input_boolean" => "Toggle helpers",
        "input_select" | "select" => "Selectors",
        "input_number" | "number" => "Numbers",
        "person" => "People",
        "device_tracker" => "Device trackers",
        "camera" => "Cameras",
        "weather" => "Weather",
        "vacuum" => "Vacuums",
        "update" => "Updates",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let spaced = domain.replace('_', " ");
    let mut letters = spaced.chars();
    letters.next().map_or(String::new(), |first| first.to_uppercase().chain(letters).collect())
}

/// Works out each entity's area: its own, or else its device's.
fn link_areas(areas: &[Value], devices: &[Value], entities: &[Value]) -> Areas {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let names: BTreeMap<String, String> = areas
        .iter()
        .filter_map(|a| Some((text(a, "area_id")?, text(a, "name")?)))
        .collect();
    let device_areas: BTreeMap<String, String> = devices
        .iter()
        .filter_map(|d| Some((text(d, "id")?, text(d, "area_id")?)))
        .collect();
    entities
        .iter()
        .filter_map(|e| {
            let id = text(e, "entity_id")?;
            let area = text(e, "area_id").or_else(|| device_areas.get(&text(e, "device_id")?).cloned())?;
            Some((id, names.get(&area)?.clone()))
        })
        .collect()
}

/// Fills in {{friendly_name}}, {{state}}, {{unit}} and any attribute by name.
pub fn fill(template: &str, entity_id: &str, entity: Option<&Entity>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let name = rest[start + 2..start + end].trim();
        let value = match (name, entity) {
            ("friendly_name", Some(e)) if !e.name.is_empty() => e.name.clone(),
            ("friendly_name", _) => entity_id.to_string(),
            (_, None) => "…".to_string(),
            ("state", Some(e)) => e.state.clone(),
            ("unit", Some(e)) => e.unit.clone(),
            (other, Some(e)) => match e.attributes.get(other) {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(value) => value.to_string(),
            },
        };
        out.push_str(&value);
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

/// The usual actions for an entity, most likely first.
pub fn services_for(entity_id: &str) -> &'static [&'static str] {
    match entity_id.split('.').next().unwrap_or("") {
        "switch" | "input_boolean" | "light" | "fan" | "cover" | "siren" | "humidifier" => {
            &["toggle", "turn_on", "turn_off"]
        }
        "automation" => &["trigger", "toggle", "turn_on", "turn_off"],
        "script" | "scene" => &["turn_on"],
        "button" | "input_button" => &["press"],
        "input_select" | "select" => &["select_next", "select_previous"],
        "media_player" => &["media_play_pause", "media_next_track", "media_previous_track", "volume_up", "volume_down"],
        "lock" => &["lock", "unlock"],
        "climate" => &["toggle", "turn_on", "turn_off"],
        _ => &[],
    }
}

/// Whether an entity can be flipped, which is what a drawn switch stands for.
/// A door sensor is on or off, but nothing can flip it.
pub fn can_toggle(entity_id: &str) -> bool {
    services_for(entity_id).contains(&"toggle")
}

/// The address for Home Assistant's web requests, from the same text.
pub fn rest_address(text: &str) -> String {
    let socket = websocket_address(text);
    let base = socket.strip_suffix("/api/websocket").unwrap_or(&socket);
    base.replacen("wss://", "https://", 1).replacen("ws://", "http://", 1)
}

/// Accepts what people usually have to hand: a browser address or the full
/// WebSocket address.
pub fn websocket_address(text: &str) -> String {
    let text = text.trim().trim_end_matches('/');
    let text = text.strip_suffix("/lovelace").unwrap_or(text);
    let with_scheme = if let Some(rest) = text.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = text.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if text.starts_with("ws://") || text.starts_with("wss://") {
        text.to_string()
    } else {
        format!("wss://{text}")
    };
    if with_scheme.ends_with("/api/websocket") {
        with_scheme
    } else {
        format!("{with_scheme}/api/websocket")
    }
}

/// What a service call needs: the entity's own domain, unless the service
/// names one, as in `notify.mobile_app`. The connection numbers it when sending.
/// With no entity, as for `homeassistant.restart`, the service runs on its own.
fn call_message(entity: &str, service: &str, data: &str) -> Result<Value, String> {
    let entity = entity.trim();
    let (domain, service) = match service.split_once('.') {
        Some((domain, service)) => (domain, service),
        None if entity.is_empty() => return Err(format!("\"{service}\" needs its domain, like homeassistant.{service}")),
        None => (entity.split('.').next().unwrap_or(""), service),
    };
    let data: Value = if data.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(data).map_err(|_| "The extra data isn't valid JSON".to_string())?
    };
    let mut message = json!({
        "type": "call_service",
        "domain": domain,
        "service": service,
        "service_data": data,
    });
    if !entity.is_empty() {
        message["target"] = json!({ "entity_id": entity });
    }
    Ok(message)
}

const KEEP_ALIVE: Duration = if cfg!(test) { Duration::from_millis(40) } else { Duration::from_secs(30) };
const REGISTRIES: [&str; 3] = [
    "config/area_registry/list",
    "config/device_registry/list",
    "config/entity_registry/list",
];

/// For check pictures: a made-up house, with no server involved.
#[cfg(any(test, feature = "shots"))]
pub fn load_example(shared: &Shared, path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let Ok(example) = serde_json::from_str::<Value>(&text) else { return };
    if let Some(states) = example.get("states").and_then(Value::as_array) {
        store_states(shared, states);
    }
    if let Some(areas) = example.get("areas").and_then(Value::as_object) {
        let areas = areas.iter().filter_map(|(id, name)| Some((id.clone(), name.as_str()?.to_string())));
        *lock(&shared.ha.areas) = areas.collect();
    }
}

/// Whether keys can show Home Assistant as it is now, or how to show that
/// they can't: while the app isn't connected, or Home Assistant is starting
/// or stopping, as during a restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Availability {
    #[default]
    Ready,
    Down {
        /// The person chose "HA offline" over the last look, dimmed.
        label: bool,
        /// A Restart Home Assistant key was pressed, and it isn't back yet.
        restarting: bool,
    },
}

/// How long a restart is taken to last at most, for the key's "Restarting…".
const RESTART_LONGEST: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, PartialEq)]
pub enum HaStatus {
    NotSet,
    Connecting,
    Connected(String),
    TokenRefused,
    Failed(String),
}

impl HaStatus {
    pub fn label(&self) -> String {
        match self {
            HaStatus::NotSet => "Not set up yet".into(),
            HaStatus::Connecting => "Connecting…".into(),
            HaStatus::Connected(version) => format!("Connected to Home Assistant {version}"),
            HaStatus::TokenRefused => "Home Assistant refused this access token".into(),
            HaStatus::Failed(why) => format!("Can't reach Home Assistant: {why}"),
        }
    }
}

pub struct Hub {
    pub status: Mutex<HaStatus>,
    /// The last states known. While Home Assistant isn't running they're kept
    /// as they were, rather than taken over by "unavailable" as it stops and
    /// starts, so keys keep their last look.
    pub entities: Mutex<Entities>,
    pub areas: Mutex<Areas>,
    /// Home Assistant itself is running, not only answering: it says
    /// `RUNNING` in `get_config`, or fired `homeassistant_started`.
    running: AtomicBool,
    /// When a Restart Home Assistant key last sent its restart.
    restarted_at: Mutex<Option<Instant>>,
    restart: Notify,
    /// Who is waiting for which answer, and the subscriptions to keep.
    pub(crate) dispatch: Arc<Dispatch>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub {
            status: Mutex::new(HaStatus::NotSet),
            entities: Mutex::new(Entities::new()),
            areas: Mutex::new(Areas::new()),
            running: AtomicBool::new(false),
            restarted_at: Mutex::new(None),
            restart: Notify::new(),
            dispatch: Default::default(),
        }
    }
}

impl Hub {
    /// Connects again, for example with a new address or token.
    pub fn reconnect(&self) {
        self.restart.notify_one();
    }

    /// Sends a message if connected, and says whether it went. Nobody waits
    /// for the answer, but a refusal is shown, as for a service call.
    pub fn send(&self, message: Value) -> bool {
        self.dispatch.post(Outgoing::Request(message, Reply::Report)).is_ok()
    }

    /// Asks Home Assistant to run a service on an entity, or on its own
    /// without one.
    pub fn call(&self, entity: &str, service: &str, data: &str) -> Result<(), String> {
        if service.trim().is_empty() {
            return Ok(());
        }
        let message = call_message(entity, service.trim(), data)?;
        if !matches!(*lock(&self.status), HaStatus::Connected(_)) {
            return Err("Home Assistant isn't connected".into());
        }
        self.dispatch.post(Outgoing::Request(message, Reply::Report)).map_err(|_| "Home Assistant isn't connected".to_string())?;
        // It closes the connection without answering; that's expected.
        if service.trim() == crate::model::RESTART_SERVICE {
            *lock(&self.restarted_at) = Some(Instant::now());
        }
        Ok(())
    }

    /// Connected, and Home Assistant itself running.
    pub fn ready(&self) -> bool {
        matches!(*lock(&self.status), HaStatus::Connected(_)) && self.running.load(Ordering::SeqCst)
    }

    /// A Restart Home Assistant key was pressed and it isn't back yet.
    pub fn restarting(&self) -> bool {
        lock(&self.restarted_at).is_some_and(|at| at.elapsed() < RESTART_LONGEST)
    }

    /// How keys should show Home Assistant now. `label` is the person's
    /// choice of "HA offline" over the last look, dimmed.
    pub fn availability(&self, label: bool) -> Availability {
        if self.ready() {
            Availability::Ready
        } else {
            Availability::Down { label, restarting: self.restarting() }
        }
    }

    /// For tests: as if connected to a running Home Assistant.
    #[cfg(test)]
    pub fn pretend_ready(&self) {
        *lock(&self.status) = HaStatus::Connected("test".into());
        self.running.store(true, Ordering::SeqCst);
    }
}

/// Home Assistant said whether it's running. Once it is, the states are
/// asked for again, as those kept while it started may be stale.
fn set_running(shared: &Arc<Shared>, running: bool) {
    let was = shared.ha.running.swap(running, Ordering::SeqCst);
    if running == was {
        return;
    }
    note(&format!("Home Assistant: {}", if running { "running" } else { "stopping or starting" }));
    if running {
        *lock(&shared.ha.restarted_at) = None;
        let keeper = shared.clone();
        let states = Reply::Then(Box::new(move |result| {
            if let Ok(Value::Array(states)) = result {
                store_states(&keeper, &states);
            }
        }));
        let _ = shared.ha.dispatch.post(Outgoing::Request(json!({ "type": "get_states" }), states));
    }
    crate::pad::wake();
    shared.touch();
}

fn set_status(shared: &Shared, status: HaStatus) {
    let mut current = lock(&shared.ha.status);
    if *current != status {
        note(&format!("Home Assistant: {}", status.label()));
        let connected = matches!(status, HaStatus::Connected(_));
        *current = status;
        drop(current);
        if !connected {
            shared.ha.running.store(false, Ordering::SeqCst);
        }
        // Keys show whether Home Assistant can be reached.
        crate::pad::wake();
        shared.touch();
        shared.repaint();
    }
}

fn store_states(shared: &Shared, states: &[Value]) {
    let fresh: Entities = states.iter().filter_map(Entity::from_json).collect();
    let mut entities = lock(&shared.ha.entities);
    if shared.ha.ready() {
        *entities = fresh;
    } else {
        // Starting: only what wasn't known yet, so keys keep their last look.
        for (id, entity) in fresh {
            entities.entry(id).or_insert(entity);
        }
    }
    drop(entities);
    crate::pad::wake();
    shared.touch();
}

/// Applies a `state_changed` event's `event` part.
fn store_change(shared: &Shared, event: &Value) {
    let data = event.get("data");
    let entity_id = data.and_then(|d| d.get("entity_id")).and_then(Value::as_str);
    let new_state = data.and_then(|d| d.get("new_state"));
    let (Some(entity_id), Some(new_state)) = (entity_id, new_state) else { return };
    let mut entities = lock(&shared.ha.entities);
    // Stopping or starting, entities go "unavailable" one by one: keys keep
    // their last look until Home Assistant runs again.
    if !shared.ha.ready() && entities.contains_key(entity_id) {
        return;
    }
    match Entity::from_json(new_state) {
        Some((id, entity)) => {
            entities.insert(id, entity);
        }
        None => {
            entities.remove(entity_id);
        }
    }
    drop(entities);
    crate::pad::wake();
    shared.touch_soon();
}

/// The subscriptions the app always has: live entity states, and the
/// command event under each of the app's names. They're made again at
/// every login for as long as the handles are kept.
fn standard_subscriptions(shared: &Arc<Shared>) -> Vec<Subscription> {
    // The handlers live inside the shared state, so they hold it loosely.
    let weak = Arc::downgrade(shared);
    let changes = shared.ha.subscribe(
        json!({ "type": "subscribe_events", "event_type": "state_changed" }),
        Box::new(move |event| {
            if let Some(shared) = weak.upgrade() {
                store_change(&shared, event);
            }
        }),
    );
    let commands = crate::remote::command_events().into_iter().enumerate().map(|(index, name)| {
        let weak = Arc::downgrade(shared);
        shared.ha.subscribe(
            json!({ "type": "subscribe_events", "event_type": name }),
            Box::new(move |event| {
                if let (Some(shared), Some(data)) = (weak.upgrade(), event.get("data")) {
                    crate::remote::hand_over(&shared, data.clone(), index);
                }
            }),
        )
    });
    let lifecycle = [("homeassistant_started", true), ("homeassistant_stop", false)].map(|(name, running)| {
        let weak = Arc::downgrade(shared);
        shared.ha.subscribe(
            json!({ "type": "subscribe_events", "event_type": name }),
            Box::new(move |_| {
                if let Some(shared) = weak.upgrade() {
                    set_running(&shared, running);
                }
            }),
        )
    });
    std::iter::once(changes).chain(commands).chain(lifecycle).collect()
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn dropped() -> HaStatus {
    HaStatus::Failed("the connection dropped".into())
}

async fn put(socket: &mut Socket, message: &Value) -> Result<(), HaStatus> {
    socket.send(Message::text(message.to_string())).await.map_err(|_| dropped())
}

/// Sends a request, numbered, with its answer going to `reply`.
async fn ask(socket: &mut Socket, wire: &mut Wire, mut message: Value, reply: Reply) -> Result<(), HaStatus> {
    wire.number(&mut message, reply);
    put(socket, &message).await
}

/// Makes a wanted subscription on this connection, if it isn't made already.
async fn subscribe(shared: &Shared, socket: &mut Socket, wire: &mut Wire, key: u64) -> Result<(), HaStatus> {
    match wire.subscription(&shared.ha.dispatch, key) {
        Some(message) => put(socket, &message).await,
        None => Ok(()),
    }
}

/// Sends what another part of the app handed over.
async fn send_out(shared: &Shared, socket: &mut Socket, wire: &mut Wire, item: Outgoing) -> Result<(), HaStatus> {
    match item {
        Outgoing::Request(message, reply) => ask(socket, wire, message, reply).await,
        Outgoing::Subscribe(key) => subscribe(shared, socket, wire, key).await,
        Outgoing::Unsubscribe(key) => match wire.unsubscription(key) {
            Some(message) => put(socket, &message).await,
            None => Ok(()),
        },
    }
}

/// What the app asks for as soon as it's logged in: every entity's state,
/// its subscriptions, and the areas, then whatever else wants a login.
async fn after_login(shared: &Arc<Shared>, socket: &mut Socket, wire: &mut Wire) -> Result<(), HaStatus> {
    // Whether Home Assistant is running or still starting, before the states,
    // which are only taken over once it runs. Answers come in order.
    let keeper = shared.clone();
    let config = Reply::Then(Box::new(move |result| {
        let state = result.ok().and_then(|config| config.get("state").and_then(Value::as_str).map(str::to_string));
        // An older Home Assistant that doesn't say is taken as running.
        let running = state.as_deref().is_none_or(|state| state == "RUNNING");
        keeper.ha.running.store(running, Ordering::SeqCst);
        if running {
            *lock(&keeper.ha.restarted_at) = None;
        }
        note(&format!("Home Assistant: {}", state.as_deref().unwrap_or("running")));
    }));
    ask(socket, wire, json!({ "type": "get_config" }), config).await?;
    let keeper = shared.clone();
    let states = Reply::Then(Box::new(move |result| {
        if let Ok(Value::Array(states)) = result {
            store_states(&keeper, &states);
        }
    }));
    ask(socket, wire, json!({ "type": "get_states" }), states).await?;
    for key in shared.ha.dispatch.wanted_keys() {
        subscribe(shared, socket, wire, key).await?;
    }
    let found: Arc<Mutex<[Option<Vec<Value>>; 3]>> = Default::default();
    for (index, kind) in REGISTRIES.iter().enumerate() {
        let (found, keeper) = (found.clone(), shared.clone());
        let registry = Reply::Then(Box::new(move |result| {
            // Areas are a help, not a need, so a refusal changes nothing.
            let mut found = lock(&found);
            found[index] = result.ok().and_then(|list| list.as_array().cloned());
            if let [Some(areas), Some(devices), Some(entities)] = &*found {
                let linked = link_areas(areas, devices, entities);
                note(&format!("Home Assistant: areas known for {} entities", linked.len()));
                *lock(&keeper.ha.areas) = linked;
                keeper.repaint();
            }
        }));
        ask(socket, wire, json!({ "type": kind }), registry).await?;
    }
    shared.ha.dispatch.logged_in(shared);
    Ok(())
}

/// One connection, from opening it to its end, which is always an error or
/// a restart; the status says which.
async fn talk(shared: &Arc<Shared>, url: &str, token: &str, outgoing: &mut UnboundedReceiver<Outgoing>, wire: &mut Wire) -> HaStatus {
    let connecting = tokio::time::timeout(Duration::from_secs(15), connect_async(url)).await;
    let mut socket = match connecting {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(e)) => return HaStatus::Failed(e.to_string()),
        Err(_) => return HaStatus::Failed("no answer".into()),
    };
    let mut version = String::new();
    let mut ping = tokio::time::interval(KEEP_ALIVE);
    ping.tick().await;
    // Anything but the login during the login counts as a bad token.
    let mut logged_in = false;
    loop {
        let step = tokio::select! {
            _ = shared.ha.restart.notified() => return HaStatus::Connecting,
            _ = ping.tick(), if logged_in => ask(&mut socket, wire, json!({ "type": "ping" }), Reply::Ignore).await,
            item = outgoing.recv(), if logged_in => match item {
                Some(item) => send_out(shared, &mut socket, wire, item).await,
                None => return HaStatus::Connecting,
            },
            incoming = socket.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => return HaStatus::Failed("the connection closed".into()),
                    Some(Err(e)) => return HaStatus::Failed(e.to_string()),
                    Some(Ok(_)) => continue,
                };
                let Ok(message) = serde_json::from_str::<Value>(&text) else { continue };
                match message.get("type").and_then(Value::as_str).unwrap_or("") {
                    "auth_required" => {
                        version = message.get("ha_version").and_then(Value::as_str).unwrap_or("").to_string();
                        put(&mut socket, &json!({ "type": "auth", "access_token": token })).await
                    }
                    "auth_invalid" => return HaStatus::TokenRefused,
                    "auth_ok" => {
                        logged_in = true;
                        set_status(shared, HaStatus::Connected(version.clone()));
                        after_login(shared, &mut socket, wire).await
                    }
                    _ => {
                        wire.receive(shared, &message);
                        Ok(())
                    }
                }
            }
        };
        if let Err(ended) = step {
            return ended;
        }
    }
}

/// Runs one connection. Whatever was still waiting for an answer when it
/// ends is told so, rather than left waiting for good.
async fn session(shared: &Arc<Shared>, url: &str, token: &str) -> HaStatus {
    let mut outgoing = shared.ha.dispatch.open();
    let mut wire = Wire::default();
    let ended = talk(shared, url, token, &mut outgoing, &mut wire).await;
    shared.ha.dispatch.close();
    outgoing.close();
    while let Ok(item) = outgoing.try_recv() {
        if let Outgoing::Request(_, reply) = item {
            reply.answer(Err(HaError::Dropped));
        }
    }
    wire.fail_all();
    ended
}

async fn run(shared: Arc<Shared>) {
    let _standard = standard_subscriptions(&shared);
    let mut wait = 2u64;
    loop {
        let (url, token) = {
            let cfg = lock(&shared.cfg);
            (websocket_address(&cfg.ha_url), unprotect(&cfg.ha_token).unwrap_or_default())
        };
        if lock(&shared.cfg).ha_url.trim().is_empty() || token.is_empty() {
            set_status(&shared, HaStatus::NotSet);
            shared.ha.restart.notified().await;
            continue;
        }
        set_status(&shared, HaStatus::Connecting);
        let started = std::time::Instant::now();
        let ended = session(&shared, &url, &token).await;
        // A connection that held for a while starts the waits over: only
        // failures in a row wait longer and longer.
        if started.elapsed() > Duration::from_secs(60) {
            wait = 2;
        }
        let refused = ended == HaStatus::TokenRefused;
        let was_restart = ended == HaStatus::Connecting;
        set_status(&shared, ended);
        if was_restart {
            wait = 2;
            continue;
        }
        if refused {
            // Trying again can't help, and would lock the token out.
            shared.ha.restart.notified().await;
            continue;
        }
        tokio::select! {
            _ = shared.ha.restart.notified() => wait = 2,
            _ = tokio::time::sleep(Duration::from_secs(wait)) => wait = (wait * 2).min(60),
        }
    }
}

/// Keeps the connection going for as long as the app runs.
pub fn start(shared: Arc<Shared>) {
    history::start(shared.clone());
    templates::start(shared.clone());
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        if let Ok(runtime) = runtime {
            runtime.block_on(run(shared));
        }
    });
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Encrypts text so only this Windows user can read it back.
pub fn protect(text: &str) -> Option<String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: text.len() as u32, pbData: text.as_ptr() as *mut u8 };
        let mut output: CRYPT_INTEGER_BLOB = std::mem::zeroed();
        let ok = CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 {
            return None;
        }
        let hex = to_hex(std::slice::from_raw_parts(output.pbData, output.cbData as usize));
        LocalFree(output.pbData as HLOCAL);
        Some(hex)
    }
}

pub fn unprotect(hex: &str) -> Option<String> {
    let mut bytes = from_hex(hex)?;
    if bytes.is_empty() {
        return None;
    }
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_mut_ptr() };
        let mut output: CRYPT_INTEGER_BLOB = std::mem::zeroed();
        let ok = CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as HLOCAL);
        String::from_utf8(text).ok()
    }
}

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod feature_tests;
