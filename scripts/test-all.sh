#!/bin/sh
# Runs every test in the repository, then builds both example apps.
# Needs: Rust, Xcode (swift, xcodebuild, lipo), Node 20+, Python 3. Leaves only git-ignored build output.
set -eu
cd "$(dirname "$0")/.."

step() { printf '\n== %s\n' "$1"; }

step "Rust: unit, integration and doc tests"
cargo test --workspace

step "ABI: Python drives every function of the real core"
cargo build -p counter-core
case "$(uname -s)" in
  Darwin) lib=target/debug/libcounter_core.dylib ;;
  *) lib=target/debug/libcounter_core.so ;;
esac
python3 tests/abi/test_abi.py "$lib"

step "Swift: CarapaceKit tests (mock backend)"
swift test

step "Swift: example app against the real core (xcframework, tests)"
(cd examples/counter/apple && ./build.sh >/dev/null && swift test)

step "TypeScript: client tests, including the real core from Node"
(cd packages/client && npm ci --silent && npm run -s typecheck && npm run -s build && npm test)

step "Tauri example: web bundle and native build"
(cd examples/counter/tauri && npm ci --silent && npm run -s typecheck && npm run -s build:web >/dev/null && cd src-tauri && cargo build -q)

printf '\nall checks passed\n'
