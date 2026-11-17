//! Requests and replies on the one Home Assistant connection.
//!
//! Home Assistant numbers everything on a connection: each message the app
//! sends carries an id, which must be higher than the last, and the answer
//! (a `result`) or a subscription's `event`s come back with the same id. This
//! keeps track of which id belongs to whom, so any part of the app can ask
//! something and get its own answer, or follow a subscription, without
//! knowing what else is going on.
//!
//! Subscriptions outlive a connection: after a reconnection they're made
//! again, under new ids, until they're cancelled.

use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Weak;
use tokio::sync::oneshot;

/// Why a request to Home Assistant got no answer.
#[derive(Clone, Debug, PartialEq)]
pub enum HaError {
    /// There's no connection to Home Assistant right now.
    NotConnected,
    /// The connection closed before Home Assistant answered.
    Dropped,
    /// Home Assistant answered with an error, such as `unknown_command`.
    Refused { code: String, message: String },
}

impl std::fmt::Display for HaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HaError::NotConnected => write!(f, "Home Assistant isn't connected"),
            HaError::Dropped => write!(f, "the connection to Home Assistant closed before it answered"),
            HaError::Refused { message, .. } => write!(f, "{message}"),
        }
    }
}

impl HaError {
    /// The error in a `result` message that says it failed.
    fn of(result: &Value) -> HaError {
        let text = |pointer: &str, otherwise: &str| result.pointer(pointer).and_then(Value::as_str).unwrap_or(otherwise).to_string();
        HaError::Refused { code: text("/error/code", ""), message: text("/error/message", "unknown error") }
    }
}

/// What gets each `event` of a subscription: the message's `event` part.
pub type EventHandler = Box<dyn Fn(&Value) + Send + Sync>;
/// What hears whether each (re)subscription was accepted.
pub type ReplyHandler = Box<dyn Fn(Result<&Value, &HaError>) + Send + Sync>;
/// Something to do each time the app has logged in to Home Assistant.
pub type LoginHook = Box<dyn Fn(&Shared) + Send + Sync>;

/// Where the answer to a request goes.
pub(crate) enum Reply {
    /// To whoever is waiting on the other end.
    Wait(oneshot::Sender<Result<Value, HaError>>),
    /// To this, run on the connection's own thread in the order answers come.
    Then(Box<dyn FnOnce(Result<Value, HaError>) + Send>),
    /// Nowhere, unless Home Assistant refused: then the person is told, as
    /// for a key's service call.
    Report,
    /// Nowhere at all.
    Ignore,
}

/// What the person is told when Home Assistant refuses a key's action.
pub(crate) fn refusal_text(code: &str, message: &str) -> String {
    if code == "unauthorized" {
        "Home Assistant refused: only an administrator can do that, and the app's access token isn't an administrator's".into()
    } else {
        format!("Home Assistant: {message}")
    }
}

impl Reply {
    fn give(self, shared: &Shared, result: Result<Value, HaError>) {
        match (self, result) {
            (Reply::Report, Err(HaError::Refused { code, message })) => {
                note(&format!("Home Assistant refused an action: {message}"));
                shared.set_status(refusal_text(&code, &message));
            }
            (reply, result) => reply.answer(result),
        }
    }

    /// Answers without telling the person anything.
    pub(crate) fn answer(self, result: Result<Value, HaError>) {
        match self {
            Reply::Wait(sender) => {
                let _ = sender.send(result);
            }
            Reply::Then(then) => then(result),
            Reply::Report | Reply::Ignore => {}
        }
    }
}

/// What the rest of the app hands the connection to send.
pub(crate) enum Outgoing {
    Request(Value, Reply),
    /// A subscription that was just added, by its key.
    Subscribe(u64),
    /// A subscription that was just cancelled, by its key.
    Unsubscribe(u64),
}

#[cfg(test)]
impl Outgoing {
    /// The message a request sends, for tests that look at what went out.
    pub(crate) fn into_message(self) -> Value {
        match self {
            Outgoing::Request(message, _) => message,
            _ => Value::Null,
        }
    }
}

/// A subscription the app wants, whether or not the connection is up.
struct Wanted {
    key: u64,
    message: Value,
    on_event: Arc<dyn Fn(&Value) + Send + Sync>,
    on_reply: Option<Arc<dyn Fn(Result<&Value, &HaError>) + Send + Sync>>,
}

/// The app's side: what to send, and the subscriptions and login hooks that
/// last from one connection to the next.
#[derive(Default)]
pub(crate) struct Dispatch {
    outbox: Mutex<Option<UnboundedSender<Outgoing>>>,
    wanted: Mutex<Vec<Wanted>>,
    keys: AtomicU64,
    login_hooks: Mutex<Vec<Arc<dyn Fn(&Shared) + Send + Sync>>>,
}

impl Dispatch {
    /// Starts taking messages for a new connection.
    pub(crate) fn open(&self) -> UnboundedReceiver<Outgoing> {
        let (sender, receiver) = unbounded_channel();
        *lock(&self.outbox) = Some(sender);
        receiver
    }

    /// Stops taking messages: the connection is gone.
    pub(crate) fn close(&self) {
        *lock(&self.outbox) = None;
    }

    /// Hands something to the connection, or gives it back if there's none.
    pub(crate) fn post(&self, item: Outgoing) -> Result<(), Outgoing> {
        match lock(&self.outbox).as_ref() {
            Some(sender) => sender.send(item).map_err(|unsent| unsent.0),
            None => Err(item),
        }
    }

    pub(crate) fn request(&self, message: Value, reply: Reply) {
        if let Err(Outgoing::Request(_, reply)) = self.post(Outgoing::Request(message, reply)) {
            reply.answer(Err(HaError::NotConnected));
        }
    }

    fn subscribe(self: &Arc<Self>, message: Value, on_event: EventHandler, on_reply: Option<ReplyHandler>) -> Subscription {
        let key = self.keys.fetch_add(1, Ordering::SeqCst) + 1;
        let on_reply = on_reply.map(Arc::from);
        lock(&self.wanted).push(Wanted { key, message, on_event: Arc::from(on_event), on_reply });
        // Without a connection, it's made at the next login.
        let _ = self.post(Outgoing::Subscribe(key));
        Subscription { key, dispatch: Arc::downgrade(self) }
    }

    fn cancel(&self, key: u64) {
        let mut wanted = lock(&self.wanted);
        let before = wanted.len();
        wanted.retain(|one| one.key != key);
        let removed = wanted.len() != before;
        drop(wanted);
        if removed {
            let _ = self.post(Outgoing::Unsubscribe(key));
        }
    }

    pub(crate) fn on_login(&self, hook: LoginHook) {
        lock(&self.login_hooks).push(Arc::from(hook));
    }

    /// Runs every login hook, after the app's own first requests.
    pub(crate) fn logged_in(&self, shared: &Shared) {
        let hooks = lock(&self.login_hooks).clone();
        for hook in hooks {
            hook(shared);
        }
    }

    /// Every subscription wanted now, in the order they were made.
    pub(crate) fn wanted_keys(&self) -> Vec<u64> {
        lock(&self.wanted).iter().map(|one| one.key).collect()
    }

    fn message_of(&self, key: u64) -> Option<Value> {
        lock(&self.wanted).iter().find(|one| one.key == key).map(|one| one.message.clone())
    }

    // The handlers run with nothing locked, so they can use the connection too.
    fn event(&self, key: u64, event: &Value) {
        let handler = lock(&self.wanted).iter().find(|one| one.key == key).map(|one| one.on_event.clone());
        if let Some(handler) = handler {
            handler(event);
        }
    }

    fn replied(&self, key: u64, result: Result<&Value, &HaError>) {
        let handler = lock(&self.wanted).iter().find(|one| one.key == key).and_then(|one| one.on_reply.clone());
        if let Some(handler) = handler {
            handler(result);
        }
    }
}

/// A subscription on the Home Assistant connection. It lasts, through
/// reconnections, until it's cancelled or dropped; then Home Assistant is
/// told to stop it.
#[must_use = "dropping a subscription cancels it"]
pub struct Subscription {
    key: u64,
    dispatch: Weak<Dispatch>,
}

impl Subscription {
    #[allow(dead_code)] // Dropping it does the same; this says so where it matters.
    pub fn cancel(self) {}
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(dispatch) = self.dispatch.upgrade() {
            dispatch.cancel(self.key);
        }
    }
}

impl Hub {
    /// Sends a message and gives Home Assistant's answer: the `result` part
    /// of its reply, or why there isn't one.
    pub fn request(&self, message: Value) -> oneshot::Receiver<Result<Value, HaError>> {
        let (sender, receiver) = oneshot::channel();
        self.dispatch.request(message, Reply::Wait(sender));
        receiver
    }

    /// Subscribes with a message such as `subscribe_events`. `handler` gets
    /// each event's `event` part, until the subscription is dropped.
    pub fn subscribe(&self, message: Value, handler: EventHandler) -> Subscription {
        self.dispatch.subscribe(message, handler, None)
    }

    /// `subscribe`, also hearing whether Home Assistant accepted it, each time
    /// it's made. A refused subscription is tried again at the next login.
    pub fn subscribe_with(&self, message: Value, handler: EventHandler, on_reply: ReplyHandler) -> Subscription {
        self.dispatch.subscribe(message, handler, Some(on_reply))
    }

    /// Runs `hook` after each successful login, once the app's own
    /// subscriptions are made again.
    pub fn on_login(&self, hook: LoginHook) {
        self.dispatch.on_login(hook);
    }
}

/// The connection's side: which ids are waiting for an answer, and which
/// belong to which subscription. It lasts as long as one connection.
#[derive(Default)]
pub(crate) struct Wire {
    /// Home Assistant refuses a message unless its number is higher than the
    /// last one, so everything sent on a connection shares this one counter.
    last_id: u64,
    pending: HashMap<u64, Reply>,
    /// Subscriptions made on this connection: their id, and their key.
    subscriptions: HashMap<u64, u64>,
}

impl Wire {
    /// Numbers a message, noting where its answer goes.
    pub(crate) fn number(&mut self, message: &mut Value, reply: Reply) {
        self.last_id += 1;
        message["id"] = json!(self.last_id);
        self.pending.insert(self.last_id, reply);
    }

    /// The message that makes a subscription, numbered, unless it's already
    /// made or no longer wanted.
    pub(crate) fn subscription(&mut self, dispatch: &Dispatch, key: u64) -> Option<Value> {
        if self.subscriptions.values().any(|made| *made == key) {
            return None;
        }
        let mut message = dispatch.message_of(key)?;
        self.last_id += 1;
        message["id"] = json!(self.last_id);
        self.subscriptions.insert(self.last_id, key);
        Some(message)
    }

    /// The message that stops a cancelled subscription, numbered, if it was made.
    pub(crate) fn unsubscription(&mut self, key: u64) -> Option<Value> {
        let id = self.subscriptions.iter().find(|(_, made)| **made == key).map(|(id, _)| *id)?;
        self.subscriptions.remove(&id);
        let mut message = json!({ "type": "unsubscribe_events", "subscription": id });
        self.number(&mut message, Reply::Ignore);
        Some(message)
    }

    /// Applies a message from Home Assistant that isn't part of the login.
    pub(crate) fn receive(&mut self, shared: &Shared, message: &Value) {
        let Some(id) = message.get("id").and_then(Value::as_u64) else { return };
        match message.get("type").and_then(Value::as_str).unwrap_or("") {
            "result" => {
                let result = if message.get("success") == Some(&Value::Bool(false)) {
                    Err(HaError::of(message))
                } else {
                    Ok(message.get("result").cloned().unwrap_or(Value::Null))
                };
                if let Some(reply) = self.pending.remove(&id) {
                    reply.give(shared, result);
                } else if let Some(&key) = self.subscriptions.get(&id) {
                    // A refused subscription doesn't exist on this connection.
                    if result.is_err() {
                        self.subscriptions.remove(&id);
                    }
                    shared.ha.dispatch.replied(key, result.as_ref());
                }
            }
            "pong" => {
                if let Some(reply) = self.pending.remove(&id) {
                    reply.answer(Ok(Value::Null));
                }
            }
            "event" => {
                if let (Some(&key), Some(event)) = (self.subscriptions.get(&id), message.get("event")) {
                    shared.ha.dispatch.event(key, event);
                }
            }
            _ => {}
        }
    }

    /// Everything still waiting is told the connection closed.
    pub(crate) fn fail_all(&mut self) {
        for (_, reply) in self.pending.drain() {
            reply.answer(Err(HaError::Dropped));
        }
    }
}
