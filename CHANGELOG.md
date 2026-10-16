# Changelog

## 0.1.1 — 2026-10-16

- Publisher and identifiers renamed to Pareto Cybernetics: licence copyright, crate authors and the
  example Tauri app identifier. No functional changes.
- Measured per-action timings in the README and `docs/architecture.md` refreshed after the runtime changes.

## 0.1.0 — 2026-10-01

First public release.

- `carapace`: `App` trait, deterministic `Engine`, threaded `Runtime` with timers and background
  work, pure `Queries`, the twelve-function C ABI and the `export!` macro.
- `carapace-codegen` and `cargo carapace gen`: typed Swift and TypeScript from the core's JSON
  Schema, with a start-up schema fingerprint check.
- `cargo carapace build apple`: universal macOS xcframework (iOS device and simulator slices with `--ios`), `doctor`.
- `CarapaceKit` and `CarapaceFFI` Swift package: `ObservableObject` store with `send`, `sendSync`,
  bindings, queries, held events and faults.
- `tauri-plugin-carapace` and `@carapace/client` (Tauri and Node transports, React hooks); the client ships as a tarball on the GitHub release.
- `examples/counter`: one core with SwiftUI, Tauri + React and Node shells; `tests/abi` in Python.
