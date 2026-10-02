# Changelog

## 0.1.0 — 2026-10-01

First public release.

- `carapace`: `App` trait, deterministic `Engine`, threaded `Runtime` with timers and background
  work, pure `Queries`, the twelve-function C ABI and the `export!` macro.
- `carapace-codegen` and `cargo carapace gen`: typed Swift and TypeScript from the core's JSON
  Schema, with a start-up schema fingerprint check.
- `cargo carapace build apple`: universal macOS xcframework (iOS slices with `--ios`), `doctor`.
- `CarapaceKit` and `CarapaceFFI` Swift package: `ObservableObject` store with `send`, `sendSync`,
  bindings, queries, held events and faults.
- `tauri-plugin-carapace` and `@carapace/client` (Tauri and Node transports, React hooks).
- `examples/counter`: one core with SwiftUI, Tauri + React and Node shells; `tests/abi` in Python.
