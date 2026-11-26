#!/bin/sh
# Rebuild the Rust core, the generated bindings and the xcframework, then the app.
set -eu
# Telemetry opt-outs for the local toolchain invoked below.
export DO_NOT_TRACK=1
export NEXT_TELEMETRY_DISABLED=1
export ASTRO_TELEMETRY_DISABLED=1
export HF_HUB_DISABLE_TELEMETRY=1
export HOMEBREW_NO_ANALYTICS=1
cd "$(dirname "$0")/.."
cargo build -q -p cargo-carapace --manifest-path ../../Cargo.toml
CARAPACE=../../target/debug/cargo-carapace
$CARAPACE gen swift -p counter-core --out apple/Sources/CounterApp
$CARAPACE build apple -p counter-core --out apple/Core
cd apple && swift build "$@"
