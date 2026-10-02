# Architecture

## The model

A core is an `App`: private state plus `update(&mut self, Action, &mut Cx)`.
After every action the runtime serialises `App::state()` to JSON. If it
differs from the last snapshot, subscribers are told. Platform requests go out
as events. Shells hold no logic beyond rendering and input.

```
Engine<A>   synchronous and deterministic: dispatch(action) -> Outcome { state?, events, effects }
Runtime<A>  one actor thread owning an Engine, plus timers and background work
ffi         the C ABI over a Runtime
```

`Engine` has no threads and no clock, so cores are unit-testable. `Runtime` is
one host for it. Another host (a WASM host driven by `setTimeout`, for example)
only has to execute the `Effect`s the engine returns.

## Guarantees

- **Order.** Actions are processed one at a time, in the order queued. `cx.send` chains run
  before the next snapshot.
- **One snapshot per dispatch.** A snapshot is published only if the JSON changed.
- **Delivery order.** For one dispatch, the state notice comes before its events.
- **Events are never lost.** Events emitted while nobody is subscribed (including from `init`)
  are kept, up to 256, and replayed to the next subscriber. The Swift store and the TypeScript
  store also hold events until a handler is set.
- **Failures are loud.** An action that does not decode, a panic in `update`, a state or event that
  does not serialise, and a `cx.send` cycle (over 10,000 hops) all become faults naming the app and
  the cause. The core keeps running after a panic.
- **Stale bindings are caught.** Generated files embed a fingerprint of the schema. The shell
  compares it with the core's at start-up.

## The C ABI

Version 1. Payloads are UTF-8 JSON. Strings returned as `char *` are owned by the caller: free
them with `carapace_string_free`.

| Function | Purpose |
|---|---|
| `carapace_abi_version`, `carapace_schema_hash`, `carapace_schema` | Identify the core and describe its types. |
| `carapace_start(config, &error)` | Start a core. `config` is a JSON object or `NULL`. |
| `carapace_dispatch(h, json, len)` | Queue an action. Returns `NULL` or an error message. |
| `carapace_dispatch_wait(h, json, len)` | Same, but returns after the action is processed and subscribers were told. |
| `carapace_query(json, len, &error)` | A pure, stateless query. Callable from any thread. |
| `carapace_state(h)` | The latest snapshot. |
| `carapace_subscribe(h, callback, user)` / `carapace_unsubscribe(h, id)` | Notifications: kind 0 state, 1 event, 2 fault. |
| `carapace_stop(h)` | Stop and join the core thread. |
| `carapace_string_free(s)` | Free a returned string. |

Rules for callers:

- The subscription callback runs on the core thread. Copy the bytes and return; hop to your UI thread.
- Never call `carapace_dispatch_wait` or `carapace_stop` from inside a callback. Both wait on the core thread.
- One core per binary: the symbols are fixed names.

## Schema and generation

`carapace::schema::<A>()` builds one JSON Schema bundle (config, state, action, event, and
optionally query and answer, with shared definitions). `cargo carapace gen` loads the built
`cdylib` (macOS and Linux) and reads it through `carapace_schema`, or reads a file with `--schema`.
Anything outside the supported shapes fails with the type path and the fix:

| Supported | Not supported (error says how to change it) |
|---|---|
| structs, string enums, internally tagged enums | externally tagged, adjacently tagged and untagged enums |
| `Option`, `Vec`, `HashMap<String, _>`, `serde_json::Value` | tuples and anonymous inline structs |
| `#[serde(default)]` (becomes a default in Swift, optional in TypeScript for inputs) | |

JSON numbers: Swift maps integers to `Int` (`UInt64` for `u64`). TypeScript maps every number to
`number`, so `i64`/`u64` values above 2^53 lose precision. Send those as strings.

On Windows, `gen` cannot load the library yet: run the core's schema example and pass `--schema`.

## State size

Every change serialises the whole state. Measured with `cargo run --release -p carapace --example bench`
on an M-series Mac, full round trip through `dispatch_wait`:

| Rows in state | Snapshot | Median | p99 |
|---|---|---|---|
| 0 | 20 B | 4.9 µs | 15.6 µs |
| 100 | 4.7 KB | 16.7 µs | 45.5 µs |
| 1,000 | 49 KB | 53.5 µs | 121 µs |
| 10,000 | 514 KB | 445 µs | 501 µs |

The shell then decodes the same bytes. Shape `State` like the view, and window long lists
(visible range plus an action to move it) instead of mirroring a whole database.

## Threads

The runtime owns one thread named `carapace-<App>`. `cx.spawn` work runs on short-lived
threads named `carapace-<App>-work`. Timers run on the core thread itself (it sleeps until the
next deadline), so a timer action and a shell action never run concurrently.
