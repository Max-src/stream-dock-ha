//! The app's side of the Stream Dock HA integration in Home Assistant
//! (docs/integration-protocol.md). On the connection the app already keeps
//! open, it says which PC this is (`connect`), keeps the integration up to
//! date (`state`), tells it about presses (`pressed`), sends the pictures of
//! the pads its dashboard cards show (`pictures`) and answers its commands
//! (`reply`).
//!
//! If the integration isn't installed, nothing changes: the app carries on as
//! before and asks again at its next login.

use crate::actions::PressSource;
use crate::ha::{HaError, HaStatus, Subscription};
use crate::model::layout::layout_of;
use crate::model::{lock, machine_hash, Config, Shared};
use crate::pad::note;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

mod pictures;
mod screens;
mod state;
pub use pictures::snapshot;
#[cfg(test)]
pub(crate) use pictures::base64;

/// The protocol version this app speaks.
pub const PROTOCOL: u64 = 1;
/// How long changes are gathered before a `state` goes out.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// The PC's CPU, memory and GPU change all the time, so on their own they're
/// sent at most this often.
const STATS_EVERY: Duration = if cfg!(test) { Duration::from_millis(600) } else { Duration::from_secs(10) };
/// A `state` goes out at least this often, whatever happens.
const AT_LEAST: Duration = if cfg!(test) { Duration::from_secs(3) } else { Duration::from_secs(60) };
/// How long the bridge sleeps with nothing to do.
const IDLE: Duration = Duration::from_secs(1);
/// While Home Assistant starts it answers before its integrations are
/// loaded, and the connection then stays, so `connect` is asked again after
/// this, then twice as long each time, up to `RETRY_MOST`.
const RETRY_FIRST: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(5) };
const RETRY_MOST: Duration = Duration::from_secs(300);
/// After this many new instance ids in a row, something else is wrong.
const MOST_RENEWALS: u32 = 3;
/// How long a new PC name or control setting stays as it is before
/// `connect` is sent again, so a name isn't sent a letter at a time.
const SETTLE: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(1) };

/// How the link to the integration is doing, for Settings > Home Assistant.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    /// The person turned it off.
    #[default]
    Off,
    /// Waiting for Home Assistant, or for its answer.
    Waiting,
    Connected,
    NotInstalled,
    UpdateIntegration,
    UpdateApp,
    NotSetUp,
    NeedsAdmin,
    Problem(String),
}

impl Status {
    /// What Settings shows. `name` is this PC's name in Home Assistant.
    pub fn label(&self, name: &str) -> String {
        match self {
            Status::Off => "Integration: turned off".into(),
            Status::Waiting => "Integration: waiting for Home Assistant".into(),
            Status::Connected => format!("Integration: connected as {name}"),
            Status::NotInstalled => "Integration: not installed in Home Assistant".into(),
            Status::UpdateIntegration => "Integration: update the Stream Dock HA integration".into(),
            Status::UpdateApp => "Integration: update this app".into(),
            Status::NotSetUp => "Integration: add Stream Dock HA in Home Assistant's Devices & services".into(),
            Status::NeedsAdmin => "The integration needs a token from an administrator user".into(),
            Status::Problem(why) => format!("Integration: {why}"),
        }
    }
}

/// What the rest of the app hands the bridge.
pub(crate) enum Note {
    /// The app logged in to Home Assistant again.
    Login,
    /// The integration's answer to `connect`.
    Connected(Result<Value, HaError>),
    /// An event on the `connect` subscription: a command or a watch.
    Event(Value),
    /// A press, as `pressed` sends it.
    Pressed(Value),
    /// The answer to something the bridge sent.
    Answer(Sent, Result<Value, HaError>),
}

/// What the bridge sent, so its answer is understood.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Sent {
    State,
    Other,
}

/// The bridge's place in the shared state.
#[derive(Default)]
pub struct Link {
    status: Mutex<Status>,
    notes: Mutex<Option<UnboundedSender<Note>>>,
    live: AtomicBool,
    /// The pads dashboard cards are watching now.
    watched: Mutex<Vec<String>>,
    /// Mini Monitor frames go over Home Assistant's local address.
    frames_local: AtomicBool,
}

impl Link {
    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    /// Whether the integration is listening now.
    pub fn is_connected(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    /// The pads dashboard cards are watching, so Mini Monitor runs for them.
    pub fn watched(&self) -> Vec<String> {
        if self.is_connected() { lock(&self.watched).clone() } else { Vec::new() }
    }

    /// Whether Mini Monitor frames for cards go over the local address.
    pub fn frames_local(&self) -> bool {
        self.frames_local.load(Ordering::SeqCst)
    }

    fn post(&self, note: Note) {
        if let Some(notes) = lock(&self.notes).as_ref() {
            let _ = notes.send(note);
        }
    }
}

/// A press, ready to tell the integration.
pub struct Press(Value);

impl Press {
    /// The press of a cell on a profile's page of a pad, as `pressed` says it.
    pub fn of(cfg: &Config, pad_id: &str, profile: usize, page: usize, slot: usize, source: PressSource, dismissed: bool) -> Option<Press> {
        let entry = cfg.pad(pad_id)?;
        let shown = cfg.profiles.get(profile)?;
        let control = layout_of(entry, shown).name_of(slot).map_or_else(|| format!("key_{}", slot + 1), str::to_string);
        let title = shown.pages.get(page).and_then(|page| page.keys.get(slot)).map_or(String::new(), |key| key.title.replace('\n', " "));
        Some(Press(json!({
            "pad_id": pad_id,
            "control": control,
            "profile": shown.name,
            "page": page + 1,
            "title": title,
            "source": source,
            "dismissed": dismissed,
        })))
    }
}

/// Tells the integration about a press, if it's listening.
pub fn pressed(shared: &Shared, press: Press) {
    if shared.bridge.is_connected() {
        shared.bridge.post(Note::Pressed(press.0));
    }
}

/// The `connect` message for these settings.
fn connect_message(cfg: &Config) -> Value {
    json!({
        "type": "stream_dock_ha/connect",
        "protocol": PROTOCOL,
        "app_version": env!("CARGO_PKG_VERSION"),
        "instance_id": cfg.instance_id,
        "machine": machine_hash(),
        "name": cfg.pc_name_or_default(),
        "control": cfg.ha_control,
    })
}

/// Hooks the bridge up to the shared state and the connection's logins, and
/// gives what it will hear, and a way to tell itself things.
pub(crate) fn link(shared: &Arc<Shared>) -> (UnboundedReceiver<Note>, UnboundedSender<Note>) {
    let (sender, receiver) = unbounded_channel();
    *lock(&shared.bridge.notes) = Some(sender.clone());
    let at_login = sender.clone();
    shared.ha.on_login(Box::new(move |_| {
        let _ = at_login.send(Note::Login);
    }));
    (receiver, sender)
}

/// Starts the bridge, for as long as the app runs.
pub fn start(shared: Arc<Shared>) {
    let (notes, back) = link(&shared);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
        if let Ok(runtime) = runtime {
            runtime.block_on(run(shared, notes, back));
        }
    });
}

/// The bridge itself: what it has told the integration, and what's due.
struct Bridge {
    shared: Arc<Shared>,
    back: UnboundedSender<Note>,
    connect: Option<Subscription>,
    /// The PC's name and the control setting that `connect` gave.
    sent_as: Option<(String, bool)>,
    /// A different name or control setting, and since when it's been so.
    changed_as: Option<((String, bool), Instant)>,
    renewals: u32,
    /// The integration accepted `connect` on this connection.
    live: bool,
    sent_state: Option<Value>,
    sent_at: Instant,
    /// Something changed at this time, and `state` may need to follow.
    dirty_since: Option<Instant>,
    /// Only the PC's numbers changed, so `state` waits until then.
    stats_due: Option<Instant>,
    pictures: pictures::Watching,
    screens: screens::Screens,
    /// When to ask `connect` again, the integration not being ready.
    retry_at: Option<Instant>,
    /// How long the last wait for that was.
    retry_wait: Duration,
}

/// Runs the bridge until the app ends.
pub(crate) async fn run(shared: Arc<Shared>, mut notes: UnboundedReceiver<Note>, back: UnboundedSender<Note>) {
    let mut changes = shared.changes.subscribe();
    let mut bridge = Bridge {
        shared,
        back,
        connect: None,
        sent_as: None,
        changed_as: None,
        renewals: 0,
        live: false,
        sent_state: None,
        sent_at: Instant::now(),
        dirty_since: None,
        stats_due: None,
        pictures: pictures::Watching::default(),
        screens: screens::Screens::default(),
        retry_at: None,
        retry_wait: Duration::ZERO,
    };
    bridge.follow_settings();
    loop {
        let wake = bridge.next_due().map_or(IDLE, |due| due.saturating_duration_since(Instant::now())).min(IDLE);
        tokio::select! {
            note = notes.recv() => match note {
                Some(note) => bridge.heard(note),
                None => return,
            },
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                bridge.changed();
            }
            _ = tokio::time::sleep(wake) => {}
        }
        bridge.tick();
    }
}

impl Bridge {
    fn set_status(&self, status: Status) {
        let mut current = lock(&self.shared.bridge.status);
        if *current != status {
            note(&format!("Home Assistant integration: {status:?}"));
            *current = status;
            drop(current);
            self.shared.repaint();
        }
    }

    fn set_live(&mut self, live: bool) {
        self.live = live;
        self.shared.bridge.live.store(live, Ordering::SeqCst);
        if !live {
            self.screens.stop();
            self.shared.bridge.frames_local.store(false, Ordering::SeqCst);
        }
    }

    /// Tells the rest of the app which pads cards are watching.
    fn share_watched(&self) {
        *lock(&self.shared.bridge.watched) = self.pictures.pad_ids();
    }

    /// Connects to the integration when the person wants it, and stops when
    /// they don't. A new PC name or control setting is sent with `connect`
    /// once it settles, so Home Assistant doesn't wait for a reconnection.
    fn follow_settings(&mut self) {
        let (wanted, now_as) = {
            let cfg = lock(&self.shared.cfg);
            (cfg.ha_integration, (cfg.pc_name_or_default(), cfg.ha_control))
        };
        if wanted && self.connect.is_none() {
            self.subscribe();
        } else if wanted && self.sent_as.as_ref() != Some(&now_as) {
            if self.changed_as.as_ref().is_none_or(|(seen, _)| *seen != now_as) {
                self.changed_as = Some((now_as, Instant::now()));
            }
        } else if !wanted && self.connect.is_some() {
            self.connect = None;
            self.stop_retrying();
            self.sent_as = None;
            self.changed_as = None;
            self.set_live(false);
            self.pictures.reset();
            self.set_status(Status::Off);
        }
    }

    /// Asks for `connect`, now if Home Assistant is connected, and again at
    /// every login for as long as it's kept.
    /// The old subscription ends after the new one is made: the integration
    /// then moves this PC to the new one instead of seeing it leave.
    fn subscribe(&mut self) {
        let message = {
            let mut cfg = lock(&self.shared.cfg);
            if !crate::model::is_id(&cfg.instance_id) {
                cfg.renew_instance();
                self.shared.save_now_with(&cfg);
            }
            connect_message(&cfg)
        };
        let name = message["name"].as_str().unwrap_or_default().to_string();
        self.sent_as = Some((name, message["control"].as_bool().unwrap_or(false)));
        self.changed_as = None;
        let (events, replies) = (self.back.clone(), self.back.clone());
        let old = self.connect.replace(self.shared.ha.subscribe_with(
            message,
            Box::new(move |event| {
                let _ = events.send(Note::Event(event.clone()));
            }),
            Box::new(move |result| {
                let _ = replies.send(Note::Connected(result.cloned().map_err(Clone::clone)));
            }),
        ));
        drop(old);
        self.set_status(Status::Waiting);
    }

    /// Sends a message, and hears its answer later as a `Note`.
    fn send(&self, message: Value, what: Sent) {
        let answer = self.shared.ha.request(message);
        let back = self.back.clone();
        tokio::spawn(async move {
            if let Ok(result) = answer.await {
                let _ = back.send(Note::Answer(what, result));
            }
        });
    }

    fn heard(&mut self, note: Note) {
        match note {
            Note::Login => {
                // The login asks `connect` again anyway.
                self.stop_retrying();
                self.set_live(false);
                self.renewals = 0;
                self.pictures.reset();
                self.sent_state = None;
                if self.connect.is_some() {
                    self.set_status(Status::Waiting);
                }
            }
            Note::Connected(result) => self.connected(result),
            Note::Event(event) => self.event(&event),
            Note::Pressed(mut press) => {
                if self.live {
                    press["type"] = json!("stream_dock_ha/pressed");
                    self.send(press, Sent::Other);
                }
            }
            Note::Answer(what, result) => self.answered(what, result),
        }
    }

    /// What the integration said to `connect`.
    fn connected(&mut self, result: Result<Value, HaError>) {
        let (code, message) = match result {
            Ok(result) => {
                let speaks = result.get("protocol").and_then(Value::as_u64).unwrap_or(PROTOCOL);
                if speaks != PROTOCOL {
                    self.mismatch(speaks);
                    return;
                }
                let version = result.get("integration_version").and_then(Value::as_str).unwrap_or("?");
                note(&format!("Home Assistant integration {version} connected"));
                self.set_live(true);
                self.screens.granted(&self.shared, screens::Grant::of(&result));
                self.stop_retrying();
                self.renewals = 0;
                self.pictures.reset();
                self.sent_state = None;
                self.dirty_since = Some(Instant::now() - DEBOUNCE);
                self.set_status(Status::Connected);
                return;
            }
            Err(HaError::Refused { code, message }) => (code, message),
            // The connection went; the next login asks again.
            Err(_) => return,
        };
        self.set_live(false);
        match code.as_str() {
            // Asked again at the next login, and in a while: Home Assistant
            // may still be loading its integrations.
            "unknown_command" => {
                self.set_status(Status::NotInstalled);
                self.retry_later();
            }
            "protocol_mismatch" => {
                note(&format!("Home Assistant integration: {message}"));
                let speaks = message.split_whitespace().filter_map(|word| word.trim_matches('.').parse::<u64>().ok()).last();
                self.mismatch(speaks.unwrap_or(0));
            }
            "renew_instance" => {
                self.renewals += 1;
                if self.renewals > MOST_RENEWALS {
                    self.set_status(Status::Problem("Home Assistant keeps refusing this PC's id".into()));
                    return;
                }
                note("Home Assistant integration: these settings came from another PC, so this PC and its pads get new ids");
                {
                    let mut cfg = lock(&self.shared.cfg);
                    cfg.renew_instance();
                    self.shared.save_now_with(&cfg);
                }
                self.subscribe();
            }
            "not_set_up" => {
                self.set_status(Status::NotSetUp);
                self.retry_later();
            }
            // Like a missing integration: asked again at the next login.
            "unauthorized" => self.set_status(Status::NeedsAdmin),
            _ => self.set_status(Status::Problem(message)),
        }
    }

    fn retry_later(&mut self) {
        self.retry_wait = (self.retry_wait * 2).clamp(RETRY_FIRST, RETRY_MOST);
        self.retry_at = Some(Instant::now() + self.retry_wait);
    }

    fn stop_retrying(&mut self) {
        self.retry_at = None;
        self.retry_wait = Duration::ZERO;
    }

    fn mismatch(&mut self, speaks: u64) {
        self.set_live(false);
        self.set_status(if speaks > PROTOCOL { Status::UpdateApp } else { Status::UpdateIntegration });
    }

    /// An event on the `connect` subscription.
    fn event(&mut self, event: &Value) {
        match event.get("kind").and_then(Value::as_str) {
            Some("command") => self.command(event),
            Some("watch") => {
                let empty = serde_json::Map::new();
                let pads = event.get("pads").and_then(Value::as_object).unwrap_or(&empty);
                self.pictures.watch(pads);
                self.share_watched();
            }
            _ => {}
        }
    }

    /// Carries out a command and answers it, always exactly once.
    fn command(&mut self, event: &Value) {
        let request_id = event.get("request_id").cloned().unwrap_or(Value::Null);
        let data = event.get("data").cloned().unwrap_or(Value::Null);
        let source = match data.get("source").and_then(Value::as_str) {
            Some("dashboard") => PressSource::Dashboard,
            _ => PressSource::Action,
        };
        if let Some(user) = data.get("user").and_then(Value::as_str) {
            note(&format!("Home Assistant command from {user}"));
        }
        let reply = match crate::remote::carry_out(&self.shared, &data, source) {
            Ok(result) => json!({ "type": "stream_dock_ha/reply", "request_id": request_id, "ok": true, "result": result }),
            Err(problem) => json!({ "type": "stream_dock_ha/reply", "request_id": request_id, "ok": false, "error": problem }),
        };
        self.send(reply, Sent::Other);
    }

    /// The answer to something the bridge sent.
    fn answered(&mut self, what: Sent, result: Result<Value, HaError>) {
        match result {
            // The integration was reloaded, or forgot this socket: connect again.
            Err(HaError::Refused { code, .. }) if code == "not_connected" => {
                if self.live {
                    note("Home Assistant integration: not connected any more, connecting again");
                    self.set_live(false);
                    self.subscribe();
                }
            }
            Err(HaError::Refused { message, .. }) => note(&format!("Home Assistant integration refused a message: {message}")),
            Err(_) => {}
            Ok(result) if what == Sent::State => self.renew_pads(&result),
            Ok(_) => {}
        }
    }

    /// Gives new ids to pads the integration knows from another PC, then
    /// sends `state` again.
    fn renew_pads(&mut self, result: &Value) {
        let Some(ids) = result.get("renew_pads").and_then(Value::as_array).filter(|ids| !ids.is_empty()) else { return };
        let mut cfg = lock(&self.shared.cfg);
        for id in ids.iter().filter_map(Value::as_str) {
            if let Some(fresh) = cfg.renew_pad(id) {
                note(&format!("Home Assistant integration: pad {id} came from another PC, it's now {fresh}"));
                self.pictures.forget(id);
            }
        }
        self.shared.save_now_with(&cfg);
        drop(cfg);
        self.sent_state = None;
        self.dirty_since = Some(Instant::now() - DEBOUNCE);
    }

    fn changed(&mut self) {
        self.follow_settings();
        self.dirty_since.get_or_insert_with(Instant::now);
        self.pictures.changed();
    }

    /// When the bridge next has something to do.
    fn next_due(&self) -> Option<Instant> {
        let reconnect = self.changed_as.as_ref().map(|(_, since)| *since + SETTLE);
        if !self.live {
            return reconnect.into_iter().chain(self.retry_at).min();
        }
        let state = [self.dirty_since.map(|since| since + DEBOUNCE), self.stats_due, Some(self.sent_at + AT_LEAST), reconnect];
        let frames = self.screens.next_due(&self.shared, &self.pictures.pad_ids());
        state.into_iter().flatten().chain(self.pictures.next_due()).chain(frames).min()
    }

    /// Sends whatever is due.
    fn tick(&mut self) {
        if self.changed_as.as_ref().is_some_and(|(_, since)| since.elapsed() >= SETTLE) {
            note("Home Assistant integration: the PC's name or control setting changed, connecting again");
            self.subscribe();
        }
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            if self.connect.is_some() && !self.live && matches!(*lock(&self.shared.ha.status), HaStatus::Connected(_)) {
                note("Home Assistant integration: asking again, as Home Assistant may have been starting");
                self.subscribe();
            }
        }
        if self.live && !matches!(*lock(&self.shared.ha.status), HaStatus::Connected(_)) {
            self.set_live(false);
        }
        if !self.live {
            return;
        }
        self.send_state_if_due();
        for message in self.pictures.due(&self.shared) {
            self.send(message, Sent::Other);
        }
        for message in self.screens.due(&self.shared, &self.pictures.pad_ids()) {
            self.send(message, Sent::Other);
        }
        self.shared.bridge.frames_local.store(self.screens.local_live(), Ordering::SeqCst);
    }

    fn send_state_if_due(&mut self) {
        let now = Instant::now();
        let changed = self.dirty_since.is_some_and(|since| now >= since + DEBOUNCE);
        let stats = self.stats_due.is_some_and(|due| now >= due);
        let periodic = now >= self.sent_at + AT_LEAST;
        if !(changed || stats || periodic || self.sent_state.is_none()) {
            return;
        }
        self.dirty_since = None;
        // Whatever was waiting is looked at now. If the PC's numbers went back
        // to what was sent, nothing is due any more; if they still differ,
        // they wait their turn again below.
        self.stats_due = None;
        let state = state::state_message(&self.shared);
        let fresh = match &self.sent_state {
            None => true,
            Some(sent) if *sent == state => periodic,
            // The PC's numbers alone wait their turn, unless it's come.
            Some(sent) if state::without_stats(sent) == state::without_stats(&state) => {
                let due = self.sent_at + STATS_EVERY;
                if now < due {
                    self.stats_due = Some(due);
                    return;
                }
                true
            }
            Some(_) => true,
        };
        if fresh {
            self.stats_due = None;
            self.sent_at = now;
            self.send(state.clone(), Sent::State);
            self.sent_state = Some(state);
        }
    }
}

#[cfg(test)]
mod tests;
