# Changelog

## 0.2.0 — 2026-11-26

- `cargo carapace` says after a command, on a terminal, when a newer release exists: one anonymous
  check of the public latest-release record at most once a day, off in CI, under agents and with
  `CARAPACE_NO_UPDATE_CHECK=1` or `update --checks off`.
- `cargo carapace update` installs the release tag with `cargo install --locked`; `--check`
  (exit 0/10/1), `--skip`, `--auto on|off|status` (off by default), `--rollback` and `--clean`.
- `cargo carapace gen` and `build apple` create generated bindings, headers and packaging inputs
  owner-only on Unix (`0700` directories, `0600` files, tightening a file that already existed);
  an existing `--out` directory keeps its permissions.
- `gen`, `build apple` and `doctor` pass telemetry opt-outs (`DO_NOT_TRACK` and friends, npm
  update notifier, audit and fund off) to every tool they start; the test and Apple example build
  scripts set them too.
- The Swift and TypeScript stores no longer copy action or query payloads into fault messages.
- A regression test fails if a telemetry or analytics package or endpoint appears in the
  workspace manifests or shipped code.
- Added [Updates](docs/updates.md) and [PRIVACY.md](PRIVACY.md), with short README Updates and
  Privacy sections. The library crates, Swift package and npm client still make no network
  requests.

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
