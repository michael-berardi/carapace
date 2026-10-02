//! Measures the core-side cost of the snapshot design.
//! Run: cargo run --release -p carapace --example bench

use std::time::Instant;

use carapace::{App, Cx, NoEvent, Runtime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, JsonSchema)]
struct Config {
    rows: usize,
}
#[derive(Serialize, JsonSchema)]
struct Row {
    id: u32,
    title: String,
    done: bool,
}
#[derive(Serialize, JsonSchema)]
struct State {
    tick: u64,
    rows: Vec<Row>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Action {
    Tick,
}
struct Bench {
    tick: u64,
    rows: Vec<Row>,
}
impl App for Bench {
    type State = State;
    type Action = Action;
    type Event = NoEvent;
    type Config = Config;
    const NAME: &'static str = "Bench";
    fn init(c: Config, _: &mut Cx<Self>) -> Self {
        let rows = (0..c.rows)
            .map(|i| Row {
                id: i as u32,
                title: format!("Task number {i}"),
                done: i % 3 == 0,
            })
            .collect();
        Bench { tick: 0, rows }
    }
    fn update(&mut self, _: Action, _: &mut Cx<Self>) {
        self.tick += 1;
    }
    fn state(&self) -> State {
        State {
            tick: self.tick,
            rows: self
                .rows
                .iter()
                .map(|r| Row {
                    id: r.id,
                    title: r.title.clone(),
                    done: r.done,
                })
                .collect(),
        }
    }
}

fn main() {
    println!(
        "{:>8} {:>12} {:>14} {:>14} {:>14}",
        "rows", "state bytes", "median RTT", "p99 RTT", "throughput"
    );
    for rows in [0usize, 10, 100, 1_000, 10_000] {
        let rt = Runtime::<Bench>::start(Config { rows });
        let bytes = rt.snapshot().len();
        let n = if rows >= 10_000 { 300 } else { 3_000 };
        let mut lat: Vec<u128> = Vec::with_capacity(n);
        for _ in 0..n {
            let t = Instant::now();
            rt.dispatch_wait(Action::Tick).unwrap();
            lat.push(t.elapsed().as_nanos());
        }
        lat.sort_unstable();
        let t = Instant::now();
        let burst = n;
        for _ in 0..burst {
            rt.dispatch(Action::Tick).unwrap();
        }
        rt.dispatch_wait(Action::Tick).unwrap();
        let per_sec = (burst as f64 + 1.0) / t.elapsed().as_secs_f64();
        println!(
            "{:>8} {:>12} {:>11.1} µs {:>11.1} µs {:>11.0} /s",
            rows,
            bytes,
            lat[n / 2] as f64 / 1e3,
            lat[n * 99 / 100] as f64 / 1e3,
            per_sec
        );
    }
}
