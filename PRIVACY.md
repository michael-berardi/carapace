# Privacy

Carapace collects nothing. There is no telemetry, account, identifier or usage data.

## Libraries

The `carapace`, `carapace-codegen` and `tauri-plugin-carapace` crates, the Swift package and
the `@carapace/client` npm package make no network requests and write no files of their own.
They run inside your app and only do what your core does.

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

## What is stored

One file, `cargo-carapace/state.json`, in your cache folder (`$XDG_CACHE_HOME`,
`~/Library/Caches` on macOS, `~/.cache` on Linux), readable only by you. It holds the last
check time and result, the latest release tag, a skipped version, when a notice was last shown,
the two update settings and the version that ran before the last update. If you turn on
automatic updates, `update.log` in the same folder records the background installs and stays
under 256 KB. `cargo carapace update --clean` removes both.

## Installing an update

`cargo carapace update` runs `cargo install` for the release tag. Cargo then fetches the
repository from GitHub and the locked dependencies from crates.io, as any `cargo install` does.
