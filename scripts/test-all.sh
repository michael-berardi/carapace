#!/bin/sh
# Runs every test in the repository, then builds both example apps.
# Needs: Rust, Xcode (swift, xcodebuild, lipo), Node 20+, Python 3. Leaves only git-ignored build output.
set -eu
# Also cap Cargo subprocesses launched by the Apple packaging CLI.
export CARGO_BUILD_JOBS=4
# Keep the local build tools quiet and offline with respect to usage reporting.
export DO_NOT_TRACK=1
export NEXT_TELEMETRY_DISABLED=1
export ASTRO_TELEMETRY_DISABLED=1
export HF_HUB_DISABLE_TELEMETRY=1
export HOMEBREW_NO_ANALYTICS=1
export npm_config_update_notifier=false npm_config_audit=false npm_config_fund=false npm_config_telemetry_enabled=false
cd "$(dirname "$0")/.."

step() { printf '\n== %s\n' "$1"; }

step "Rust: unit, integration and doc tests"
cargo test --workspace -j 4

step "ABI: Python drives every function of the real core"
cargo build -j 4 -p counter-core
case "$(uname -s)" in
  Darwin) lib=target/debug/libcounter_core.dylib ;;
  *) lib=target/debug/libcounter_core.so ;;
esac
python3 Tests/abi/test_abi.py "$lib"

step "Swift: CarapaceKit tests (mock backend)"
swift test -j 4

step "Swift: example app against the real core (xcframework, tests)"
(cd examples/counter/apple && ./build.sh -j 4 >/dev/null && swift test -j 4)

step "TypeScript: client tests, including the real core from Node"
(cd packages/client && npm ci --silent && npm run -s typecheck && npm run -s build && npm test)

step "Tauri example: web bundle and native build"
(cd examples/counter/tauri && npm ci --silent && npm run -s typecheck && npm run -s build:web >/dev/null && cd src-tauri && cargo build -j 4 -q)

printf '\nall checks passed\n'
