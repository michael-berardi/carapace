//! The threaded host: one actor thread owns the [`Engine`], runs timers and
//! background work, and fans snapshots and events out to subscribers.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::engine::{App, Effect, Engine, Outcome, Queries, Repeat};

/// Events emitted while nobody is subscribed are kept (up to this many) and
/// replayed to the next subscriber, so events from `init` are never lost.
const BACKLOG_CAP: usize = 256;

/// Something a subscriber is told about. Strings are JSON (faults are text).
#[derive(Debug, Clone, Copy)]
pub enum Notice<'a> {
    State(&'a str),
    Event(&'a str),
    Fault(&'a str),
}

#[derive(Debug)]
pub enum Error {
    /// The action JSON did not match the app's `Action` schema.
    Decode {
        app: &'static str,
        message: String,
        input: String,
    },
    /// The runtime has been shut down.
    Stopped(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Decode {
                app,
                message,
                input,
            } => {
                write!(
                    f,
                    "{app}: cannot decode action ({message}); received: {input}"
                )
            }
            Error::Stopped(app) => write!(f, "{app}: the core has been shut down"),
        }
    }
}

impl std::error::Error for Error {}

type Subscriber = Arc<dyn Fn(Notice<'_>) + Send + Sync>;

enum Msg<A: App> {
    Action(A::Action, Option<SyncSender<()>>),
    Stop,
}

struct Shared<A: App> {
    tx: Mutex<Sender<Msg<A>>>,
    snapshot: RwLock<Arc<str>>,
    subs: Mutex<Subs>,
}

#[derive(Default)]
struct Subs {
    next: u64,
    list: Vec<(u64, Subscriber)>,
    backlog: VecDeque<String>,
}

/// A cloneable, thread-safe way to dispatch actions to a running core.
pub struct Handle<A: App> {
    shared: Arc<Shared<A>>,
}

impl<A: App> Clone for Handle<A> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<A: App> Handle<A> {
    /// Queue an action. Returns once it is queued, not once it is processed.
    pub fn dispatch(&self, action: A::Action) -> Result<(), Error> {
        self.shared
            .tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(Msg::Action(action, None))
            .map_err(|_| Error::Stopped(A::NAME))
    }

    /// Queue an action and wait until it is processed and subscribers were told.
    /// Never call this from a subscriber callback (it runs on the core thread).
    pub fn dispatch_wait(&self, action: A::Action) -> Result<Arc<str>, Error> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.shared
            .tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(Msg::Action(action, Some(ack_tx)))
            .map_err(|_| Error::Stopped(A::NAME))?;
        ack_rx.recv().map_err(|_| Error::Stopped(A::NAME))?;
        Ok(self.snapshot())
    }

    /// Decode an action from JSON, queue it, and wait until it is processed and subscribers
    /// were told. Never call this from a subscriber callback (it runs on the core thread).
    pub fn dispatch_json_wait(&self, json: &str) -> Result<Arc<str>, Error> {
        self.dispatch_wait(decode::<A>(json)?)
    }

    /// Decode an action from JSON and queue it.
    pub fn dispatch_json(&self, json: &str) -> Result<(), Error> {
        self.dispatch(decode::<A>(json)?)
    }

    /// The latest state snapshot, readable from any thread without blocking the core.
    pub fn snapshot(&self) -> Arc<str> {
        self.shared
            .snapshot
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Subscribe to state, events and faults. The callback runs on the core
    /// thread: keep it short and hand work to your UI thread.
    pub fn subscribe(&self, f: impl Fn(Notice<'_>) + Send + Sync + 'static) -> u64 {
        let f: Subscriber = Arc::new(f);
        let (id, replay) = {
            let mut subs = self.shared.subs.lock().unwrap_or_else(|p| p.into_inner());
            subs.next += 1;
            let id = subs.next;
            subs.list.push((id, f.clone()));
            (id, std::mem::take(&mut subs.backlog))
        };
        for event in replay {
            f(Notice::Event(&event));
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        self.shared
            .subs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .list
            .retain(|(i, _)| *i != id);
    }
}

/// Run a pure query from JSON to JSON. Errors name the app and show the offending input.
pub fn query_json<A: Queries>(json: &str) -> Result<String, String> {
    let q: A::Query = serde_json::from_str(json).map_err(|e| {
        let shown: String = json.chars().take(200).collect();
        format!("{}: cannot decode query ({e}); received: {shown}", A::NAME)
    })?;
    serde_json::to_string(&A::query(q))
        .map_err(|e| format!("{}: answer is not serialisable to JSON: {e}", A::NAME))
}

fn decode<A: App>(json: &str) -> Result<A::Action, Error> {
    serde_json::from_str(json).map_err(|e| Error::Decode {
        app: A::NAME,
        message: e.to_string(),
        input: json.chars().take(200).collect(),
    })
}

/// A running core: the actor thread plus its [`Handle`].
pub struct Runtime<A: App> {
    handle: Handle<A>,
    thread: Option<JoinHandle<()>>,
}

impl<A: App> Runtime<A> {
    /// Start the app on its own thread.
    pub fn start(config: A::Config) -> Self {
        let (engine, first) = Engine::<A>::start(config);
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            tx: Mutex::new(tx),
            snapshot: RwLock::new(Arc::from(engine.snapshot())),
            subs: Mutex::new(Subs::default()),
        });
        let handle = Handle { shared };
        let mut actor = Actor {
            engine,
            rx,
            handle: handle.clone(),
            timers: Vec::new(),
        };
        // Publish what `init` produced before returning, so its events are in the
        // backlog and its timers are armed by the time the caller can subscribe.
        actor.publish(first);
        let thread = thread::Builder::new()
            .name(format!("carapace-{}", A::NAME))
            .spawn(move || actor.run())
            .expect("carapace: cannot spawn the core thread");
        Self {
            handle,
            thread: Some(thread),
        }
    }

    pub fn handle(&self) -> Handle<A> {
        self.handle.clone()
    }
}

impl<A: App> std::ops::Deref for Runtime<A> {
    type Target = Handle<A>;
    fn deref(&self) -> &Handle<A> {
        &self.handle
    }
}

impl<A: App> Drop for Runtime<A> {
    fn drop(&mut self) {
        let _ = self
            .handle
            .shared
            .tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Timer<A: App> {
    key: &'static str,
    due: Instant,
    repeat: Repeat,
    action: A::Action,
}

struct Actor<A: App> {
    engine: Engine<A>,
    rx: Receiver<Msg<A>>,
    handle: Handle<A>,
    timers: Vec<Timer<A>>,
}

impl<A: App> Actor<A> {
    fn run(mut self) {
        loop {
            let next_due = self.timers.iter().map(|t| t.due).min();
            let msg = match next_due {
                None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(due) => self
                    .rx
                    .recv_timeout(due.saturating_duration_since(Instant::now())),
            };
            match msg {
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(Msg::Action(action, ack)) => {
                    self.apply(action);
                    if let Some(ack) = ack {
                        let _ = ack.send(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => self.fire_due_timers(),
            }
        }
    }

    fn fire_due_timers(&mut self) {
        let now = Instant::now();
        let mut due = Vec::new();
        self.timers.retain_mut(|t| {
            if t.due > now {
                return true;
            }
            due.push(t.action.clone());
            match t.repeat {
                Repeat::Once(_) => false,
                Repeat::Every(every) => {
                    t.due = now + every;
                    true
                }
            }
        });
        for action in due {
            self.apply(action);
        }
    }

    fn apply(&mut self, action: A::Action) {
        match catch_unwind(AssertUnwindSafe(|| self.engine.dispatch(action))) {
            Ok(outcome) => self.publish(outcome),
            Err(panic) => {
                let what = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "non-string panic".into());
                self.notify(Notice::Fault(&format!(
                    "{}: update panicked: {what}",
                    A::NAME
                )));
            }
        }
    }

    fn publish(&mut self, outcome: Outcome<A>) {
        if let Some(state) = &outcome.state {
            *self
                .handle
                .shared
                .snapshot
                .write()
                .unwrap_or_else(|p| p.into_inner()) = Arc::from(state.as_str());
            self.notify(Notice::State(state));
        }
        for event in &outcome.events {
            match serde_json::to_string(event) {
                Ok(json) => self.deliver_event(json),
                Err(e) => self.notify(Notice::Fault(&format!(
                    "{}: event is not serialisable to JSON: {e}",
                    A::NAME
                ))),
            }
        }
        if let Some(fault) = &outcome.fault {
            self.notify(Notice::Fault(fault));
        }
        for effect in outcome.effects {
            match effect {
                Effect::Schedule {
                    key,
                    repeat,
                    action,
                } => {
                    self.timers.retain(|t| t.key != key);
                    let wait = match repeat {
                        Repeat::Once(d) | Repeat::Every(d) => d,
                    };
                    self.timers.push(Timer {
                        key,
                        due: Instant::now() + wait,
                        repeat,
                        action,
                    });
                }
                Effect::Cancel { key } => self.timers.retain(|t| t.key != key),
                Effect::Spawn(work) => {
                    let handle = self.handle.clone();
                    let spawned = thread::Builder::new()
                        .name(format!("carapace-{}-work", A::NAME))
                        .spawn(move || work(handle));
                    if let Err(e) = spawned {
                        self.notify(Notice::Fault(&format!(
                            "{}: cannot spawn background work: {e}",
                            A::NAME
                        )));
                    }
                }
            }
        }
    }

    fn deliver_event(&self, json: String) {
        let subs: Vec<Subscriber> = {
            let mut subs = self
                .handle
                .shared
                .subs
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if subs.list.is_empty() {
                if subs.backlog.len() >= BACKLOG_CAP {
                    subs.backlog.pop_front();
                }
                subs.backlog.push_back(json);
                return;
            }
            subs.list.iter().map(|(_, f)| f.clone()).collect()
        };
        for f in subs {
            f(Notice::Event(&json));
        }
    }

    fn notify(&self, notice: Notice<'_>) {
        let subs: Vec<Subscriber> = self
            .handle
            .shared
            .subs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .list
            .iter()
            .map(|(_, f)| f.clone())
            .collect();
        for f in subs {
            // A misbehaving subscriber must not take the core down.
            let _ = catch_unwind(AssertUnwindSafe(|| f(notice)));
        }
    }
}
