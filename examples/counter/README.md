# Counter

One Rust core, three shells.

| Path | Shell |
|---|---|
| `src/lib.rs` | The core: state, actions, a timer, background work, an event and a pure query. |
| `apple/` | SwiftUI window and menu bar item. `./build.sh` regenerates the bindings, builds the xcframework and the app; `swift test` runs the real core. |
| `tauri/` | Tauri 2 + React. `npm run build:web` then `cargo build` in `src-tauri`. `COUNTER_START=42 ./target/debug/counter-tauri` shows config reaching the core. |
| `web/Counter.ts` | Generated TypeScript, used by the Node tests in `packages/client`. |

Regenerate bindings after changing the core: `cargo carapace gen swift|ts -p counter-core --out <dir>`.
