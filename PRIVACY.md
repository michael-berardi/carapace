# Privacy

Carapace collects nothing. There is no telemetry, analytics, crash reporter, beacon, account,
identifier or usage data, and no user or machine identifiers in the code.

## Libraries

The `carapace`, `carapace-codegen` and `tauri-plugin-carapace` crates, the Swift package and
the `@carapace/client` npm package make no network requests, open no sockets and write no files
of their own. They run inside your app and only do what your core does.

- State, events and faults stay in memory. Fault history is capped at the newest 50 messages,
  and fault messages never copy action or query payloads, which can hold user data.
- Nothing is written to a user data directory. Quit the app to discard the in-memory data.
- The example windows load only local CSS, scripts and images: no remote fonts or CDNs.
- Carapace configures no third-party service. If you write a core or shell that calls one, that
  service receives whatever your code sends it.

## The `cargo carapace` release check

The CLI makes one request to see whether a newer release exists:

- `GET https://api.github.com/repos/michael-berardi/carapace/releases/latest`, the public
  release record of the repository named in the CLI's own package metadata.
- Headers: a fixed `User-Agent: cargo-carapace-update-check` and `Accept`. No token, cookie,
  installed version, machine data or usage data. Your `.curlrc` is not read.
- At most once every 24 hours, after a command has finished, and only when stdout and stderr
  are a terminal. Never in CI, under agent hosts (`CLAUDECODE`, `CODEX_SANDBOX`,
  `GEMINI_CLI`, `CARAPACE_AGENT`) or when turned off.
- GitHub sees the request like any web request (your IP address and the time). Nothing else
  is sent anywhere.

Turn it off with `CARAPACE_NO_UPDATE_CHECK=1` or `cargo carapace update --checks off`.
`cargo carapace update` and `update --check` always check, because you asked.

## Local build tools

`cargo carapace gen`, `build apple` and `doctor` run local tools only (`cargo`, `rustc`,
`rustup`, `lipo`, `xcodebuild`, `swift`, `node`, `python3`) and send nothing themselves. Every tool they start gets
telemetry opt-outs (`DO_NOT_TRACK=1`, `NEXT_TELEMETRY_DISABLED=1`, `ASTRO_TELEMETRY_DISABLED=1`,
`HF_HUB_DISABLE_TELEMETRY=1`, `HOMEBREW_NO_ANALYTICS=1`, and npm's update notifier, audit, fund
and telemetry turned off), even when your shell does not set them. `cargo` itself may still
download a crate your project is missing. The repository's test script and the Apple example's
build script set the same opt-outs. The install commands in the README download the client from
GitHub, or dependencies from npm and crates.io, because you run them.

## What is stored

The release check keeps one file, `cargo-carapace/state.json`, in your cache folder
(`$XDG_CACHE_HOME`, `~/Library/Caches` on macOS, `~/.cache` on Linux), readable only by you. It
holds the last check time and result, the latest release tag, a skipped version, when a notice
was last shown, the two update settings and the version that ran before the last update. If you
turn on automatic updates, `update.log` in the same folder records the background installs and
stays under 256 KB. `cargo carapace update --clean` removes both.

Generated output goes only where you ask, because it can mirror your own types and source:

- `cargo carapace gen` writes Swift and TypeScript bindings to `./generated` (or `--out`).
- `cargo carapace build apple` writes an xcframework under `./apple` (or `--out`) and its
  intermediate libraries under the Cargo target folder (`carapace-apple`).
- On Unix, directories the CLI creates are `0700` and files it writes are `0600`, including a
  file that already existed with looser permissions. An existing directory you pass as `--out`
  keeps the permissions you gave it.

Delete those output folders (and `cargo clean`) to remove generated files. There is no other
stored data to clear.

## Installing an update

`cargo carapace update` runs `cargo install` for the release tag. Cargo then fetches the
repository from GitHub and the locked dependencies from crates.io, as any `cargo install` does.
