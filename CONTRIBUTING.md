# Contributing

Small, focused pull requests are easiest to review.

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
scripts/test-all.sh     # Rust, Swift, TypeScript and ABI tests, plus both example apps build
```

CI is local: run the checks before opening a PR. `scripts/test-all.sh` needs a Mac with Xcode,
Node 20+ and Python 3. It starts no long-running processes and leaves nothing behind except
build output, which is git-ignored.

## Wanted

- Windows support for `cargo carapace gen` (read the schema from a built `.dll`)
- A Kotlin generator and a Dart generator, tested against a real Android or Flutter build
- A WASM host for `Engine`, so a core can run in a browser tab with no native library
- iOS verification of the `build apple --ios` slices

## Ground rules

- Keep the ABI small and stable. A new symbol needs a reason that no shell can solve with an
  action, an event or a query.
- Nothing fails silently. Every error names the app, the cause and, where there is one, the fix.
- Generated code must compile. Add a case to `crates/carapace-codegen/src/tests.rs` for every
  schema shape you handle, and run the Swift example to prove it compiles.
