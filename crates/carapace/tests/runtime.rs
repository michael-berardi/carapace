use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use carapace::{App, Cx, Handle, Notice, Runtime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct Config {}

#[derive(Serialize, JsonSchema)]
struct State {
    n: i64,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Action {
    Add { by: i64 },
    Loop,
    Boom,
    Later,
    Every,
    Stop,
    Work,
    Done { value: i64 },
    Shout,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Event {
    Hello,
    Shout { text: String },
}

struct T(i64);

impl App for T {
    type State = State;
    type Action = Action;
    type Event = Event;
    type Config = Config;
    const NAME: &'static str = "T";
    fn init(_: Config, cx: &mut Cx<Self>) -> Self {
        cx.emit(Event::Hello);
        T(0)
    }
    fn update(&mut self, a: Action, cx: &mut Cx<Self>) {
        match a {
            Action::Add { by } => self.0 += by,
            Action::Loop => cx.send(Action::Loop),
            Action::Boom => panic!("kaboom"),
            Action::Later => cx.after("later", Duration::from_millis(40), Action::Add { by: 10 }),
            Action::Every => cx.every("every", Duration::from_millis(20), Action::Add { by: 1 }),
            Action::Stop => cx.cancel("every"),
            Action::Work => cx.spawn(|h: Handle<T>| {
                std::thread::sleep(Duration::from_millis(10));
                h.dispatch(Action::Done { value: 77 }).unwrap();
            }),
            Action::Done { value } => self.0 = value,
            Action::Shout => cx.emit(Event::Shout { text: "hi".into() }),
        }
    }
    fn state(&self) -> State {
        State { n: self.0 }
    }
}

fn n(rt: &Runtime<T>) -> i64 {
    serde_json::from_str::<serde_json::Value>(&rt.snapshot()).unwrap()["n"]
        .as_i64()
        .unwrap()
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "timed out waiting for {what}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn dispatch_wait_returns_the_new_snapshot() {
    let rt = Runtime::<T>::start(Config {});
    let s = rt.dispatch_wait(Action::Add { by: 4 }).unwrap();
    assert_eq!(&*s, r#"{"n":4}"#);
}

#[test]
fn init_events_replay_to_the_first_subscriber() {
    let rt = Runtime::<T>::start(Config {});
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    rt.subscribe(move |n| {
        if let Notice::Event(e) = n {
            s.lock().unwrap().push(e.to_string());
        }
    });
    assert_eq!(seen.lock().unwrap().as_slice(), [r#"{"type":"hello"}"#]);
}

#[test]
fn undecodable_json_names_the_app_and_the_input() {
    let rt = Runtime::<T>::start(Config {});
    let err = rt
        .dispatch_json(r#"{"type":"nope"}"#)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("T: cannot decode action") && err.contains("nope"),
        "{err}"
    );
}

#[test]
fn a_panic_in_update_is_a_fault_and_the_core_keeps_running() {
    let rt = Runtime::<T>::start(Config {});
    let faults = Arc::new(Mutex::new(vec![]));
    let f = faults.clone();
    rt.subscribe(move |n| {
        if let Notice::Fault(t) = n {
            f.lock().unwrap().push(t.to_string());
        }
    });
    rt.dispatch(Action::Boom).unwrap();
    rt.dispatch_wait(Action::Add { by: 1 }).unwrap();
    assert_eq!(n(&rt), 1);
    let faults = faults.lock().unwrap();
    assert!(faults[0].contains("kaboom"), "{faults:?}");
}

#[test]
fn a_send_cycle_is_a_fault_not_a_hang() {
    let rt = Runtime::<T>::start(Config {});
    let faults = Arc::new(Mutex::new(vec![]));
    let f = faults.clone();
    rt.subscribe(move |n| {
        if let Notice::Fault(t) = n {
            f.lock().unwrap().push(t.to_string());
        }
    });
    rt.dispatch_wait(Action::Loop).unwrap();
    assert!(faults.lock().unwrap()[0].contains("cycle"));
}

#[test]
fn after_fires_once_and_replaces_by_key() {
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch(Action::Later).unwrap();
    rt.dispatch(Action::Later).unwrap();
    wait_until("the timer", || n(&rt) == 10);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(n(&rt), 10, "the replaced timer must not also fire");
}

#[test]
fn every_repeats_until_cancelled() {
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch(Action::Every).unwrap();
    wait_until("three ticks", || n(&rt) >= 3);
    rt.dispatch_wait(Action::Stop).unwrap();
    let at_stop = n(&rt);
    std::thread::sleep(Duration::from_millis(100));
    assert!(n(&rt) <= at_stop + 1);
}

#[test]
fn spawned_work_reports_back_through_the_handle() {
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch(Action::Work).unwrap();
    wait_until("background result", || n(&rt) == 77);
}

#[test]
fn unsubscribe_stops_delivery() {
    let rt = Runtime::<T>::start(Config {});
    let count = Arc::new(Mutex::new(0));
    let c = count.clone();
    let id = rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            *c.lock().unwrap() += 1;
        }
    });
    rt.dispatch_wait(Action::Add { by: 1 }).unwrap();
    rt.unsubscribe(id);
    rt.dispatch_wait(Action::Add { by: 1 }).unwrap();
    assert_eq!(*count.lock().unwrap(), 1);
}

#[test]
fn dispatch_after_drop_reports_stopped() {
    let rt = Runtime::<T>::start(Config {});
    let h = rt.handle();
    drop(rt);
    assert!(h
        .dispatch(Action::Shout)
        .unwrap_err()
        .to_string()
        .contains("shut down"));
}

#[test]
fn schema_hash_is_stable_and_matches_the_codegen() {
    let schema = carapace::schema::<T>();
    let a = carapace::schema_hash(&schema);
    assert_eq!(a, carapace::schema_hash(&carapace::schema::<T>()));
    assert_eq!(a, carapace_codegen::schema_hash(&schema));
}
