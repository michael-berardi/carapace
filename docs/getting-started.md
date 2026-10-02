# Getting started

This walks through one core and three shells. The finished versions live in
[`examples/counter`](../examples/counter).

## 1. Write the core

A core is a Rust type that implements `App`. Its types must follow two serde
conventions, which the generators rely on and check:

- enums used as `Action`, `Event` or `Query` are internally tagged:
  `#[serde(tag = "type", rename_all = "camelCase")]`
- structs use `#[serde(rename_all = "camelCase")]`

```rust
use std::time::Duration;
use carapace::{App, Cx};

impl App for Counter {
    type State = State;     // what the UI renders
    type Action = Action;   // what the UI can ask for
    type Event = Event;     // requests to the platform (or NoEvent)
    type Config = Config;   // start-up parameters from the shell
    const NAME: &'static str = "Counter";

    fn init(config: Config, cx: &mut Cx<Self>) -> Self { /* ... */ }

    fn update(&mut self, action: Action, cx: &mut Cx<Self>) {
        match action {
            Action::StartTicking => cx.every("ticker", Duration::from_millis(250), Action::Tick),
            Action::StopTicking => cx.cancel("ticker"),
            Action::Fetch => cx.spawn(|handle| {
                let value = slow_work();
                let _ = handle.dispatch(Action::Fetched { value });
            }),
            Action::Fetched { value } => {
                self.count = value;
                cx.emit(Event::Notify { title: "Fetched".into(), body: format!("{value}") });
            }
            // ...
        }
    }

    fn state(&self) -> State { /* ... */ }
}
```

`Cx` is how the core asks for things:

| Call | Effect |
|---|---|
| `cx.emit(event)` | A request to the shell (notify, open a URL, haptics). |
| `cx.send(action)` | Process another action right after this one, before the next snapshot. |
| `cx.after(key, delay, action)` / `cx.every(key, interval, action)` | Timers. A later call with the same key replaces the pending one (debounce in one line). |
| `cx.cancel(key)` | Cancel a timer. |
| `cx.spawn(\|handle\| ...)` | Run blocking work on a background thread; dispatch the result back through the handle. |

Test the logic without any UI:

```rust
let (mut engine, _) = carapace::Engine::<Counter>::start(Config::default());
let out = engine.dispatch(Action::Increment);
assert!(out.state.unwrap().contains(r#""count":1"#));
```

Export it. Both lines below are needed in the crate that builds the library:

```rust
carapace::export!(Counter);             // or carapace::export!(Counter, queries);
```

```toml
[lib]
crate-type = ["lib", "staticlib", "cdylib"]
```

### Pure queries

Some things a shell needs are not state: colour math while animating, parsing,
formatting. Implement `Queries` and export with `queries`:

```rust
impl carapace::Queries for Counter {
    type Query = Query;     // internally tagged enum
    type Answer = Answer;
    fn query(q: Query) -> Answer { /* a pure function */ }
}
carapace::export!(Counter, queries);
```

Queries are stateless, run on the caller's thread, and are safe to call every frame.

## 2. SwiftUI (macOS, and iOS slices)

```sh
cargo install --git https://github.com/michael-berardi/carapace cargo-carapace
cargo carapace gen swift -p my-core --out App/Sources/App      # MyCore.swift
cargo carapace build apple -p my-core --out App/Core           # MyCore.xcframework (macOS arm64+x86_64)
```

`Package.swift`:

```swift
dependencies: [.package(url: "https://github.com/michael-berardi/carapace", from: "0.1.0")],
targets: [
    .binaryTarget(name: "MyCore", path: "Core/MyCore.xcframework"),
    .executableTarget(name: "App", dependencies: [
        .product(name: "CarapaceKit", package: "carapace"),
        .product(name: "CarapaceFFI", package: "carapace"),
        "MyCore",
    ]),
]
```

Use it:

```swift
let store = try Store<Counter>(backend: RustBackend(), config: .init(start: 10))

store.state.count                         // read; the store is an ObservableObject
store.send(.increment)                    // queue an action
store.sendSync(.setStep(step: 5))         // apply the result before returning (use inside withAnimation)
store.binding(\.step) { .setStep(step: $0) }   // a SwiftUI Binding
store.onEvent = { event in ... }          // platform requests; held until set, never dropped
try store.query(.describe(value: 3))      // pure query
store.faults                              // panics and rejected actions, newest last
```

`Store` is `@MainActor`. Notifications from the core thread are coalesced into
one main-thread hop: bursts of state changes decode once. The `CarapaceApp`
conformance and all types come from the generated file. Rerun `gen` whenever
the core's types change; a stale file throws `CarapaceError.staleBindings`
naming the command to run.

The macOS deployment target is 13. `build apple --ios` adds iOS device and
simulator slices (install the Rust targets first). Run `cargo carapace doctor`
to see what is missing.

## 3. Tauri 2 (Windows, Linux, macOS)

Rust side:

```toml
tauri-plugin-carapace = { git = "https://github.com/michael-berardi/carapace" }
```

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_carapace::plugin::<MyApp, _>(Default::default()))
    // or plugin_with_queries::<MyApp, _>(config)
```

Add `"carapace:default"` to `src-tauri/capabilities/default.json`.

Web side:

```sh
# the package is attached to each GitHub release (registry listing pending)
npm i https://github.com/michael-berardi/carapace/releases/download/v0.1.0/carapace-client-0.1.0.tgz
cargo carapace gen ts -p my-core --out src/generated
```

```tsx
import { Store } from "@carapace/client";
import { tauriTransport } from "@carapace/client/tauri";
import { useCarapace, useCarapaceEvents } from "@carapace/client/react";
import { actions, appName, schemaHash, type Types } from "./generated/MyApp";

const store = await Store.connect<Types>(tauriTransport(), { schemaHash, appName });

function Counter() {
  const s = useCarapace(store);
  return <button onClick={() => store.dispatch(actions.increment())}>{s.count}</button>;
}
```

`store.query(...)`, `store.onEvent(...)` and `store.faults` mirror the Swift API.
`dispatch` rejects with the core's own message when an action cannot be decoded.
Without React, use `store.subscribe` and `store.getSnapshot` directly.

If you bundle with a linked local copy of `@carapace/client`, alias `react` to a single
copy (`esbuild --alias:react=./node_modules/react`); two copies of React break hooks.

## 4. Node and Electron

```ts
import { Store } from "@carapace/client";
import { nodeTransport } from "@carapace/client/node";   // needs `npm i koffi`

const transport = await nodeTransport("./target/release/libmy_core.dylib", { config: { start: 10 } });
const store = await Store.connect<Types>(transport, { schemaHash });
```

This loads the `cdylib` through the C ABI. It is the transport the test suite
runs against the real core. The same code path serves an Electron main process.

## 5. Anything else

Load the `cdylib` and call the functions in [`carapace.h`](../Sources/CCarapace/include/carapace.h).
[`tests/abi/test_abi.py`](../tests/abi/test_abi.py) is a complete, working caller in about 110 lines.
