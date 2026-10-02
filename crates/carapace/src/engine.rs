//! The deterministic heart of Carapace: an [`App`] plus a synchronous [`Engine`]
//! that turns one action into a state snapshot, events and effect requests.
//!
//! The engine never touches threads, clocks or the outside world, so an app's
//! logic is testable with plain `#[test]`s. Hosts (the threaded
//! [`Runtime`](crate::Runtime), or one you write) run the effects it requests.

use std::collections::VecDeque;
use std::time::Duration;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Maximum chained `cx.send` calls inside one dispatch. A cycle trips this and
/// becomes a reported fault instead of a hang.
pub const MAX_CHAIN: usize = 10_000;

/// An application core. Implement this once; every shell drives the same code.
///
/// Serde conventions the generators rely on (violations are reported with the
/// exact type and fix by `cargo carapace gen`):
/// - `Action` and `Event` enums use `#[serde(tag = "type", rename_all = "camelCase")]`.
/// - Structs use `#[serde(rename_all = "camelCase")]`.
pub trait App: Send + Sized + 'static {
    /// Everything the UI renders. Serialised to JSON after every change.
    type State: Serialize + JsonSchema;
    /// Everything the UI can ask for.
    type Action: DeserializeOwned + Serialize + JsonSchema + Clone + Send + 'static;
    /// One-way requests to the shell (notify, open a URL, haptics). Use
    /// [`NoEvent`] when the core never needs the platform.
    type Event: Serialize + JsonSchema + Send + 'static;
    /// Start-up parameters from the shell (data directory, locale, flags).
    type Config: DeserializeOwned + Serialize + JsonSchema + Default + Send;

    /// Name of the app; becomes the namespace in generated code.
    const NAME: &'static str;

    fn init(config: Self::Config, cx: &mut Cx<Self>) -> Self;
    fn update(&mut self, action: Self::Action, cx: &mut Cx<Self>);
    fn state(&self) -> Self::State;
}

/// Pure, stateless helpers a shell can call synchronously, for example colour math it
/// needs while animating, parsing, or formatting. They never touch app state and run on
/// the caller's thread, so they are cheap and cannot race the core.
///
/// Export with `carapace::export!(MyApp, queries)`.
pub trait Queries: App {
    type Query: DeserializeOwned + Serialize + JsonSchema;
    type Answer: Serialize + DeserializeOwned + JsonSchema;

    fn query(query: Self::Query) -> Self::Answer;
}

/// Event type for apps whose core never asks the platform for anything.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "type")]
pub enum NoEvent {}

/// How a timer repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Once(Duration),
    Every(Duration),
}

/// An effect the core asked for. Hosts execute these.
pub enum Effect<A: App> {
    /// Fire `action` on the schedule; replaces any timer with the same key.
    Schedule {
        key: &'static str,
        repeat: Repeat,
        action: A::Action,
    },
    /// Cancel the timer with this key, if any.
    Cancel { key: &'static str },
    /// Run blocking or long work off the core thread. The closure receives a
    /// handle to dispatch the result back as an action.
    Spawn(Box<dyn FnOnce(crate::Handle<A>) + Send>),
}

/// The context handed to `init` and `update` to request events and effects.
pub struct Cx<A: App> {
    pub(crate) events: Vec<A::Event>,
    pub(crate) effects: Vec<Effect<A>>,
    pub(crate) chained: VecDeque<A::Action>,
}

impl<A: App> Cx<A> {
    pub(crate) fn new() -> Self {
        Self {
            events: Vec::new(),
            effects: Vec::new(),
            chained: VecDeque::new(),
        }
    }

    /// Ask the shell to do something on the platform.
    pub fn emit(&mut self, event: A::Event) {
        self.events.push(event);
    }

    /// Process `action` right after the current one, before any new snapshot.
    pub fn send(&mut self, action: A::Action) {
        self.chained.push_back(action);
    }

    /// Dispatch `action` once after `delay`. A later call with the same key
    /// replaces the pending timer, which makes debouncing a one-liner.
    pub fn after(&mut self, key: &'static str, delay: Duration, action: A::Action) {
        self.effects.push(Effect::Schedule {
            key,
            repeat: Repeat::Once(delay),
            action,
        });
    }

    /// Dispatch `action` every `interval` until cancelled or replaced.
    pub fn every(&mut self, key: &'static str, interval: Duration, action: A::Action) {
        self.effects.push(Effect::Schedule {
            key,
            repeat: Repeat::Every(interval),
            action,
        });
    }

    /// Cancel a timer by key.
    pub fn cancel(&mut self, key: &'static str) {
        self.effects.push(Effect::Cancel { key });
    }

    /// Run `work` on a background thread. Dispatch results back through the handle.
    pub fn spawn(&mut self, work: impl FnOnce(crate::Handle<A>) + Send + 'static) {
        self.effects.push(Effect::Spawn(Box::new(work)));
    }
}

/// What one dispatch produced.
pub struct Outcome<A: App> {
    /// New state JSON, present only when it differs from the previous snapshot.
    pub state: Option<String>,
    pub events: Vec<A::Event>,
    pub effects: Vec<Effect<A>>,
    /// Set when the chain limit was hit.
    pub fault: Option<String>,
}

/// Synchronous, deterministic driver of an [`App`].
pub struct Engine<A: App> {
    app: A,
    snapshot: String,
}

impl<A: App> Engine<A> {
    /// Start the app. Returns the engine and everything `init` requested.
    pub fn start(config: A::Config) -> (Self, Outcome<A>) {
        let mut cx = Cx::new();
        let app = A::init(config, &mut cx);
        let mut engine = Self {
            app,
            snapshot: String::new(),
        };
        let outcome = engine.settle(cx);
        (engine, outcome)
    }

    /// Apply one action and everything it chains.
    pub fn dispatch(&mut self, action: A::Action) -> Outcome<A> {
        let mut cx = Cx::new();
        self.app.update(action, &mut cx);
        self.settle(cx)
    }

    /// Re-serialise the state after an `update` panicked part-way, so subscribers see what the
    /// app actually holds now. Returns the new snapshot if it changed.
    pub fn resync(&mut self) -> Option<String> {
        let json = serde_json::to_string(&self.app.state()).ok()?;
        (json != self.snapshot).then(|| {
            self.snapshot = json.clone();
            json
        })
    }

    /// The current snapshot as JSON.
    pub fn snapshot(&self) -> &str {
        &self.snapshot
    }

    fn settle(&mut self, mut cx: Cx<A>) -> Outcome<A> {
        let mut fault = None;
        let mut hops = 0usize;
        while let Some(next) = cx.chained.pop_front() {
            hops += 1;
            if hops > MAX_CHAIN {
                fault = Some(format!(
                    "{}: more than {MAX_CHAIN} chained cx.send calls in one dispatch; an action sends itself in a cycle",
                    A::NAME
                ));
                cx.chained.clear();
                break;
            }
            self.app.update(next, &mut cx);
        }
        let json = serde_json::to_string(&self.app.state())
            .unwrap_or_else(|e| panic!("{}: state is not serialisable to JSON: {e}", A::NAME));
        let changed = json != self.snapshot;
        if changed {
            self.snapshot = json.clone();
        }
        Outcome {
            state: changed.then_some(json),
            events: cx.events,
            effects: cx.effects,
            fault,
        }
    }
}
