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
    MutateThenPanic,
    Forever,
    Fast,
    ArmPair,
    CancelB,
    B,
    SlowEvery,
    SlowTick,
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
            Action::MutateThenPanic => {
                self.0 = 500;
                panic!("half done");
            }
            Action::Forever => cx.after("forever", Duration::MAX, Action::Add { by: 1 }),
            Action::Fast => cx.every("fast", Duration::ZERO, Action::Add { by: 1 }),
            Action::ArmPair => {
                cx.after("a", Duration::from_millis(20), Action::CancelB);
                cx.after("b", Duration::from_millis(20), Action::B);
            }
            Action::CancelB => cx.cancel("b"),
            Action::B => self.0 += 1000,
            Action::SlowEvery => cx.every("slow", Duration::from_millis(10), Action::SlowTick),
            Action::SlowTick => {
                std::thread::sleep(Duration::from_millis(40));
                self.0 += 1;
            }
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

#[test]
fn unsubscribe_waits_for_a_callback_in_flight() {
    let rt = Runtime::<T>::start(Config {});
    let finished = Arc::new(Mutex::new(false));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let f = finished.clone();
    let id = rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            entered_tx.send(()).ok();
            std::thread::sleep(Duration::from_millis(150));
            *f.lock().unwrap() = true;
        }
    });
    rt.dispatch(Action::Add { by: 1 }).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    rt.unsubscribe(id);
    assert!(
        *finished.lock().unwrap(),
        "unsubscribe returned while the callback was still running"
    );
}

#[test]
fn dropping_the_runtime_from_a_callback_does_not_deadlock_or_panic() {
    let slot: Arc<Mutex<Option<Runtime<T>>>> = Arc::new(Mutex::new(None));
    let rt = Runtime::<T>::start(Config {});
    let s = slot.clone();
    rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            // Drops the last owner on the core thread itself.
            s.lock().unwrap().take();
        }
    });
    let h = rt.handle();
    *slot.lock().unwrap() = Some(rt);
    h.dispatch(Action::Add { by: 1 }).unwrap();
    wait_until("the core to stop", || {
        h.dispatch(Action::Add { by: 1 }).is_err()
    });
}

#[test]
fn dispatch_wait_from_a_callback_is_an_error_not_a_hang() {
    let rt = Runtime::<T>::start(Config {});
    let h = rt.handle();
    let result = Arc::new(Mutex::new(None));
    let r = result.clone();
    rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            *r.lock().unwrap() = Some(
                h.dispatch_wait(Action::Add { by: 1 })
                    .unwrap_err()
                    .to_string(),
            );
        }
    });
    rt.dispatch_wait(Action::Add { by: 1 }).unwrap();
    assert!(result
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .contains("would wait on itself"));
}

#[test]
fn an_absurd_timer_delay_does_not_kill_the_core() {
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch_wait(Action::Forever).unwrap();
    rt.dispatch_wait(Action::Add { by: 2 }).unwrap();
    assert_eq!(n(&rt), 2);
}

#[test]
fn a_zero_interval_repeat_is_clamped_instead_of_spinning() {
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch(Action::Fast).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    rt.dispatch_wait(Action::Stop).unwrap();
    rt.dispatch_wait(Action::Add { by: 0 }).unwrap();
    assert!(n(&rt) <= 110, "{} ticks in 100 ms", n(&rt));
}

#[test]
fn a_timer_cancelled_by_an_earlier_due_timer_does_not_fire() {
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch(Action::ArmPair).unwrap();
    std::thread::sleep(Duration::from_millis(120));
    rt.dispatch_wait(Action::Add { by: 0 }).unwrap();
    assert_eq!(n(&rt), 0, "B fired although A cancelled it first");
}

#[test]
fn a_panic_mid_update_publishes_the_real_state_and_fails_dispatch_wait() {
    let rt = Runtime::<T>::start(Config {});
    let states = Arc::new(Mutex::new(vec![]));
    let s = states.clone();
    rt.subscribe(move |n| {
        if let Notice::State(t) = n {
            s.lock().unwrap().push(t.to_string());
        }
    });
    let err = rt
        .dispatch_wait(Action::MutateThenPanic)
        .unwrap_err()
        .to_string();
    assert!(err.contains("half done"), "{err}");
    assert_eq!(n(&rt), 500);
    assert_eq!(states.lock().unwrap().last().unwrap(), r#"{"n":500}"#);
}

#[test]
fn state_fields_skipped_when_serialising_are_optional_in_the_schema() {
    // Regression: the schema used to be built from serde's deserialize side, which marks a
    // `skip_serializing_if` field required, so a shell would fail to decode a state without it.
    #[derive(serde::Serialize, JsonSchema)]
    struct S {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        errors: Vec<String>,
        n: i32,
    }
    struct Skippy;
    impl App for Skippy {
        type State = S;
        type Action = Action;
        type Event = Event;
        type Config = Config;
        const NAME: &'static str = "Skippy";
        fn init(_: Config, _: &mut Cx<Self>) -> Self {
            Skippy
        }
        fn update(&mut self, _: Action, _: &mut Cx<Self>) {}
        fn state(&self) -> S {
            S {
                errors: vec![],
                n: 1,
            }
        }
    }
    let schema = carapace::schema::<Skippy>();
    let required = &schema["definitions"]["S"]["required"];
    assert_eq!(required, &serde_json::json!(["n"]), "{schema}");
}

#[test]
fn a_timer_slower_than_its_interval_cannot_starve_actions_or_stop() {
    // Regression: the timer pass used to loop while anything was due, so a tick longer than its
    // interval kept the core in the pass forever and no queued action or Stop ever ran.
    let rt = Runtime::<T>::start(Config {});
    rt.dispatch(Action::SlowEvery).unwrap();
    std::thread::sleep(Duration::from_millis(60));
    let started = Instant::now();
    rt.dispatch_wait(Action::Add { by: 1000 }).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "waited {:?}",
        started.elapsed()
    );
    let started = Instant::now();
    drop(rt);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "stop took {:?}",
        started.elapsed()
    );
}

#[test]
fn replay_callbacks_may_call_back_into_the_runtime_without_deadlocking() {
    let rt = Runtime::<T>::start(Config {});
    let h = rt.handle();
    let waited = Arc::new(Mutex::new(None));
    let w = waited.clone();
    let inner_subscribed = Arc::new(Mutex::new(false));
    let i = inner_subscribed.clone();
    // The core emitted Hello during init: it is replayed to this first subscriber.
    rt.subscribe(move |n| {
        if let Notice::Event(_) = n {
            *w.lock().unwrap() = Some(
                h.dispatch_wait(Action::Add { by: 1 })
                    .unwrap_err()
                    .to_string(),
            );
            h.subscribe(|_| {});
            *i.lock().unwrap() = true;
        }
    });
    assert!(waited
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .contains("would wait on itself"));
    assert!(*inner_subscribed.lock().unwrap());
}

#[test]
fn no_callback_starts_after_the_runtime_was_dropped_from_inside_one() {
    let slot: Arc<Mutex<Option<Runtime<T>>>> = Arc::new(Mutex::new(None));
    let rt = Runtime::<T>::start(Config {});
    let after = Arc::new(Mutex::new(0usize));
    let s = slot.clone();
    rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            s.lock().unwrap().take();
        }
    });
    let a = after.clone();
    rt.subscribe(move |_| *a.lock().unwrap() += 1);
    let h = rt.handle();
    *slot.lock().unwrap() = Some(rt);
    h.dispatch(Action::Add { by: 1 }).unwrap();
    wait_until("the core to stop", || {
        h.dispatch(Action::Add { by: 1 }).is_err()
    });
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        *after.lock().unwrap(),
        0,
        "the second subscriber was called after stop"
    );
}

#[test]
#[should_panic(expected = "two different types are both named")]
fn two_different_types_with_the_same_name_in_both_directions_are_rejected() {
    mod shell {
        #[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema, Default)]
        pub struct Dup {
            pub x: i32,
        }
    }
    mod core {
        #[derive(serde::Serialize, schemars::JsonSchema)]
        pub struct Dup {
            pub y: String,
        }
    }
    #[derive(Default, serde::Serialize, serde::Deserialize, JsonSchema)]
    struct Cfg {
        d: shell::Dup,
    }
    #[derive(serde::Serialize, JsonSchema)]
    struct St {
        d: core::Dup,
    }
    struct Clash;
    impl App for Clash {
        type State = St;
        type Action = Action;
        type Event = Event;
        type Config = Cfg;
        const NAME: &'static str = "Clash";
        fn init(_: Cfg, _: &mut Cx<Self>) -> Self {
            Clash
        }
        fn update(&mut self, _: Action, _: &mut Cx<Self>) {}
        fn state(&self) -> St {
            St {
                d: core::Dup { y: String::new() },
            }
        }
    }
    let _ = carapace::schema::<Clash>();
}

#[test]
fn a_subscriber_unsubscribed_from_another_callback_is_not_called_again() {
    // Regression: the fan-out had already copied the list, so B was still called after A removed it.
    let rt = Runtime::<T>::start(Config {});
    let h = rt.handle();
    let b_calls = Arc::new(Mutex::new(0usize));
    let b_id = Arc::new(Mutex::new(0u64));
    let (hh, bid) = (h.clone(), b_id.clone());
    rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            hh.unsubscribe(*bid.lock().unwrap());
        }
    });
    let bc = b_calls.clone();
    *b_id.lock().unwrap() = rt.subscribe(move |n| {
        if let Notice::State(_) = n {
            *bc.lock().unwrap() += 1;
        }
    });
    rt.dispatch_wait(Action::Add { by: 1 }).unwrap();
    rt.dispatch_wait(Action::Add { by: 1 }).unwrap();
    assert_eq!(*b_calls.lock().unwrap(), 0);
}

#[test]
fn dropping_the_runtime_from_a_replay_callback_does_not_deadlock() {
    // Regression: drop joined the core thread while the replaying thread held the delivery lock,
    // and the core thread was waiting for that lock to deliver its next notice.
    let rt = Runtime::<T>::start(Config {});
    let h = rt.handle();
    let slot: Arc<Mutex<Option<Runtime<T>>>> = Arc::new(Mutex::new(Some(rt)));
    h.dispatch(Action::Add { by: 1 }).unwrap(); // the core is about to deliver a state notice
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let s = slot.clone();
        // The init event (Hello) replays to this first subscriber, on this thread, under the delivery lock.
        h.subscribe(move |n| {
            if let Notice::Event(_) = n {
                s.lock().unwrap().take();
            }
        });
        done_tx.send(()).unwrap();
    });
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("dropping from a replay callback deadlocked");
}
