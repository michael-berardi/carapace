#!/bin/sh
# Rebuild the Rust core, the generated bindings and the xcframework, then the app.
set -eu
cd "$(dirname "$0")/.."
cargo build -q -p cargo-carapace --manifest-path ../../Cargo.toml
CARAPACE=../../target/debug/cargo-carapace
$CARAPACE gen swift -p counter-core --out apple/Sources/CounterApp
$CARAPACE build apple -p counter-core --out apple/Core
cd apple && swift build "$@"
