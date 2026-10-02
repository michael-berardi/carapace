//! A small but complete Carapace core: state, actions, timers, background work
//! and an event that asks the platform to do something.

use std::time::Duration;

use carapace::{App, Cx, Queries};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Where the counter starts.
    #[serde(default)]
    pub start: i64,
}

#[derive(Serialize, JsonSchema, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Idle,
    Ticking,
    Fetching,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub count: i64,
    pub step: u32,
    pub mode: Mode,
    pub history: Vec<i64>,
    pub label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    Increment,
    Decrement,
    SetStep {
        step: u32,
    },
    Rename {
        label: Option<String>,
    },
    StartTicking,
    StopTicking,
    Tick,
    /// Pretend to do slow work off the core thread, then report back.
    Fetch,
    Fetched {
        value: i64,
    },
    Reset,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    /// The shell should show a notification.
    Notify { title: String, body: String },
}

pub struct Counter {
    count: i64,
    step: u32,
    mode: Mode,
    history: Vec<i64>,
    label: Option<String>,
}

impl Counter {
    fn bump(&mut self, delta: i64) {
        self.count += delta;
        self.history.push(self.count);
        if self.history.len() > 8 {
            self.history.remove(0);
        }
    }
}

impl App for Counter {
    type State = State;
    type Action = Action;
    type Event = Event;
    type Config = Config;
    const NAME: &'static str = "Counter";

    fn init(config: Config, cx: &mut Cx<Self>) -> Self {
        cx.emit(Event::Notify {
            title: "Counter".into(),
            body: "Core started".into(),
        });
        Counter {
            count: config.start,
            step: 1,
            mode: Mode::Idle,
            history: vec![],
            label: None,
        }
    }

    fn update(&mut self, action: Action, cx: &mut Cx<Self>) {
        match action {
            Action::Increment => self.bump(i64::from(self.step)),
            Action::Decrement => self.bump(-i64::from(self.step)),
            Action::SetStep { step } => self.step = step.max(1),
            Action::Rename { label } => self.label = label.filter(|l| !l.is_empty()),
            Action::StartTicking => {
                self.mode = Mode::Ticking;
                cx.every("ticker", Duration::from_millis(250), Action::Tick);
            }
            Action::StopTicking => {
                self.mode = Mode::Idle;
                cx.cancel("ticker");
            }
            Action::Tick => self.bump(1),
            Action::Fetch => {
                self.mode = Mode::Fetching;
                cx.spawn(|handle| {
                    std::thread::sleep(Duration::from_millis(150));
                    let _ = handle.dispatch(Action::Fetched { value: 100 });
                });
            }
            Action::Fetched { value } => {
                self.mode = Mode::Idle;
                self.count = value;
                self.history.push(value);
                cx.emit(Event::Notify {
                    title: "Fetched".into(),
                    body: format!("Count is now {value}"),
                });
            }
            Action::Reset => {
                self.count = 0;
                self.history.clear();
                cx.cancel("ticker");
                self.mode = Mode::Idle;
            }
        }
    }

    fn state(&self) -> State {
        State {
            count: self.count,
            step: self.step,
            mode: self.mode.clone(),
            history: self.history.clone(),
            label: self.label.clone(),
        }
    }
}

/// Pure helpers the shell can call without going through state.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Query {
    /// Describe a number in words the UI can show.
    Describe { value: i64 },
}

#[derive(Serialize, Deserialize, JsonSchema, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub text: String,
}

impl Queries for Counter {
    type Query = Query;
    type Answer = Answer;

    fn query(query: Query) -> Answer {
        match query {
            Query::Describe { value } => {
                let parity = if value % 2 == 0 { "even" } else { "odd" };
                let sign = match value.signum() {
                    -1 => "negative",
                    0 => "zero",
                    _ => "positive",
                };
                Answer {
                    text: format!("{sign}, {parity}"),
                }
            }
        }
    }
}

carapace::export!(Counter, queries);

#[cfg(test)]
mod tests {
    use super::*;
    use carapace::Engine;

    #[test]
    fn step_applies_and_history_is_bounded() {
        let (mut e, _) = Engine::<Counter>::start(Config::default());
        e.dispatch(Action::SetStep { step: 5 });
        for _ in 0..12 {
            e.dispatch(Action::Increment);
        }
        let v: serde_json::Value = serde_json::from_str(e.snapshot()).unwrap();
        assert_eq!(v["count"], 60);
        assert_eq!(v["history"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn query_describes_numbers() {
        assert_eq!(
            Counter::query(Query::Describe { value: -3 }).text,
            "negative, odd"
        );
        assert_eq!(
            Counter::query(Query::Describe { value: 0 }).text,
            "zero, even"
        );
    }

    #[test]
    fn unchanged_state_produces_no_snapshot() {
        let (mut e, _) = Engine::<Counter>::start(Config::default());
        assert!(e.dispatch(Action::Rename { label: None }).state.is_none());
    }
}
