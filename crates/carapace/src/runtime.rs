//! The threaded host: one actor thread owns the [`Engine`], runs timers and
//! background work, and fans snapshots and events out to subscribers.

use std::collections::VecDeque;
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::{Duration, Instant};

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
    /// A blocking call was made from the core thread (inside a subscriber callback), where it
    /// could never finish.
    Reentrant {
        app: &'static str,
        call: &'static str,
    },
    /// `update` panicked while handling an action waited on with `dispatch_wait`.
    Panicked { app: &'static str, message: String },
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
            Error::Reentrant { app, call } => write!(
                f,
                "{app}: {call} was called from the core thread (a subscriber callback) and would wait on itself; hand the work to another thread"
            ),
            Error::Panicked { app, message } => write!(f, "{app}: update panicked: {message}"),
        }
    }
}

impl std::error::Error for Error {}

type Subscriber = Arc<dyn Fn(Notice<'_>) + Send + Sync>;

enum Msg<A: App> {
    Action(A::Action, Option<SyncSender<Result<(), String>>>),
    Stop,
}

/// Serialises subscriber callbacks and remembers which thread is inside one, so a callback
/// that calls back into the runtime (subscribe, unsubscribe, dispatch_wait) is recognised
/// instead of deadlocking on a lock its own thread already holds.
#[derive(Default)]
struct DeliveryLock {
    lock: Mutex<()>,
    holder: Mutex<Option<ThreadId>>,
}

struct DeliveryGuard<'a> {
    owner: &'a DeliveryLock,
    _guard: MutexGuard<'a, ()>,
}

impl DeliveryLock {
    fn held_by_current_thread(&self) -> bool {
        *self.holder.lock().unwrap_or_else(|p| p.into_inner()) == Some(thread::current().id())
    }

    /// Take the lock, or `None` when this thread already holds it (a re-entrant call).
    fn lock_unless_held(&self) -> Option<DeliveryGuard<'_>> {
        if self.held_by_current_thread() {
            return None;
        }
        let guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        *self.holder.lock().unwrap_or_else(|p| p.into_inner()) = Some(thread::current().id());
        Some(DeliveryGuard {
            owner: self,
            _guard: guard,
        })
    }
}

impl Drop for DeliveryGuard<'_> {
    fn drop(&mut self) {
        *self.owner.holder.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

struct Shared<A: App> {
    tx: Mutex<Sender<Msg<A>>>,
    snapshot: RwLock<Arc<str>>,
    subs: Mutex<Subs>,
    /// Held while subscribers are being called. `unsubscribe` takes it, so once it returns no
    /// callback for that subscriber is running or will start.
    delivery: DeliveryLock,
    /// Set when the runtime is dropped: no callback starts after this.
    stopped: AtomicBool,
    /// The core thread, to detect calls that would wait on themselves.
    core_thread: OnceLock<ThreadId>,
}

impl<A: App> Shared<A> {
    fn on_core_thread(&self) -> bool {
        self.core_thread.get() == Some(&thread::current().id())
    }
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

    /// Queue an action and wait until it is processed and subscribers were told. Fails with
    /// [`Error::Panicked`] if `update` panicked on it, and with [`Error::Reentrant`] when called
    /// from a subscriber callback (which runs on the core thread).
    pub fn dispatch_wait(&self, action: A::Action) -> Result<Arc<str>, Error> {
        if self.shared.on_core_thread() || self.shared.delivery.held_by_current_thread() {
            return Err(Error::Reentrant {
                app: A::NAME,
                call: "dispatch_wait",
            });
        }
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.shared
            .tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(Msg::Action(action, Some(ack_tx)))
            .map_err(|_| Error::Stopped(A::NAME))?;
        match ack_rx.recv().map_err(|_| Error::Stopped(A::NAME))? {
            Ok(()) => Ok(self.snapshot()),
            Err(message) => Err(Error::Panicked {
                app: A::NAME,
                message,
            }),
        }
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

    /// Subscribe to state, events and faults. Callbacks run on the core thread, never two at
    /// once; keep them short and hand work to your UI thread. Never block them on another
    /// thread that may be calling `unsubscribe` or dropping the runtime. Events emitted before
    /// the first subscription are replayed to it, in order, on the subscribing thread, before
    /// any newer notification. A callback may itself call `subscribe`, `unsubscribe` or `dispatch`.
    pub fn subscribe(&self, f: impl Fn(Notice<'_>) + Send + Sync + 'static) -> u64 {
        let f: Subscriber = Arc::new(f);
        // Holds the delivery lock for the replay, unless this thread is already inside a callback.
        let _delivery = self.shared.delivery.lock_unless_held();
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

    /// Stop notifications to `id`. When this returns (from any thread that is not inside a
    /// callback), no callback for it is running and none will start, so the callback and its
    /// `user` data can be freed. From inside a callback it returns at once.
    pub fn unsubscribe(&self, id: u64) {
        self.shared
            .subs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .list
            .retain(|(i, _)| *i != id);
        drop(self.shared.delivery.lock_unless_held());
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
            delivery: DeliveryLock::default(),
            stopped: AtomicBool::new(false),
            core_thread: OnceLock::new(),
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
        let _ = handle.shared.core_thread.set(thread.thread().id());
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
        // No callback starts after this, even if we were dropped from inside one.
        self.handle.shared.stopped.store(true, Ordering::SeqCst);
        let _ = self
            .handle
            .shared
            .tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            // Dropped from inside a callback: the core thread cannot join itself. It sees Stop next.
            if !self.handle.shared.on_core_thread() {
                let _ = t.join();
            }
        }
    }
}

/// Timers never wait longer than this: far enough to mean "never", close enough that
/// `Instant` arithmetic cannot overflow.
const FOREVER: Duration = Duration::from_secs(60 * 60 * 24 * 365 * 30);
/// A repeating timer faster than this would spin the core.
const MIN_INTERVAL: Duration = Duration::from_millis(1);

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

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic".into())
}

impl<A: App> Actor<A> {
    fn run(mut self) {
        let _ = self.handle.shared.core_thread.set(thread::current().id());
        loop {
            // Overdue timers run before anything else, so a steady stream of actions cannot starve them.
            self.fire_due_timers();
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
                    let result = self.apply(action);
                    if let Some(ack) = ack {
                        let _ = ack.send(result);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// One pass over the timers that are due right now: each fires at most once, earliest first,
    /// and only if an earlier handler in the pass did not cancel or replace it. The pass is bounded
    /// by the number of timers, so a slow tick cannot starve queued actions, `Stop` or `dispatch_wait`.
    fn fire_due_timers(&mut self) {
        let now = Instant::now();
        let mut due: Vec<(Instant, &'static str)> = self
            .timers
            .iter()
            .filter(|t| t.due <= now)
            .map(|t| (t.due, t.key))
            .collect();
        due.sort();
        for (planned, key) in due {
            // Cancelled or replaced by an earlier handler in this pass?
            let Some(i) = self
                .timers
                .iter()
                .position(|t| t.key == key && t.due == planned)
            else {
                continue;
            };
            let action = self.timers[i].action.clone();
            match self.timers[i].repeat {
                Repeat::Once(_) => {
                    self.timers.remove(i);
                }
                Repeat::Every(every) => {
                    // From the planned time, so the interval does not drift. If the tick ran
                    // late, the next one is due now: it then waits behind queued actions.
                    let every = every.max(MIN_INTERVAL);
                    let t = &mut self.timers[i];
                    t.due = (t.due + every).max(Instant::now());
                }
            }
            let _ = self.apply(action);
        }
    }

    /// Run one action. `Err` carries the panic message when `update` panicked.
    fn apply(&mut self, action: A::Action) -> Result<(), String> {
        match catch_unwind(AssertUnwindSafe(|| self.engine.dispatch(action))) {
            Ok(outcome) => {
                self.publish(outcome);
                Ok(())
            }
            Err(panic) => {
                let what = panic_message(panic);
                self.notify(Notice::Fault(&format!(
                    "{}: update panicked: {what}",
                    A::NAME
                )));
                // The app may have changed before it panicked: show subscribers what it holds now.
                if let Ok(Some(state)) = catch_unwind(AssertUnwindSafe(|| self.engine.resync())) {
                    self.publish_state(&state);
                }
                Err(what)
            }
        }
    }

    fn publish_state(&mut self, state: &str) {
        *self
            .handle
            .shared
            .snapshot
            .write()
            .unwrap_or_else(|p| p.into_inner()) = Arc::from(state);
        self.notify(Notice::State(state));
    }

    fn publish(&mut self, outcome: Outcome<A>) {
        if let Some(state) = &outcome.state {
            self.publish_state(state);
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
                        Repeat::Once(d) => d,
                        Repeat::Every(d) => d.max(MIN_INTERVAL),
                    };
                    // An absurd delay means "never", not a panic on overflow.
                    let due = Instant::now()
                        .checked_add(wait.min(FOREVER))
                        .unwrap_or_else(|| Instant::now() + FOREVER);
                    self.timers.push(Timer {
                        key,
                        due,
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
        let _delivery = self.handle.shared.delivery.lock_unless_held();
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
            if self.handle.shared.stopped.load(Ordering::SeqCst) {
                return;
            }
            // A misbehaving subscriber must not take the core down.
            let _ = catch_unwind(AssertUnwindSafe(|| f(Notice::Event(&json))));
        }
    }

    fn notify(&self, notice: Notice<'_>) {
        let _delivery = self.handle.shared.delivery.lock_unless_held();
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
            if self.handle.shared.stopped.load(Ordering::SeqCst) {
                return;
            }
            let _ = catch_unwind(AssertUnwindSafe(|| f(notice)));
        }
    }
}
