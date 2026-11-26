<!-- update-contract: v1 -->
# Updates

`cargo carapace` tells you when a newer stable release exists and installs it in one step.
Only the CLI checks. The Rust crates, the Swift package and the npm client are libraries and
make no network requests; update them through your own `Cargo.toml`, `Package.swift` or
`package.json`.

## The notice

After a command has finished, on a terminal, the CLI may print two lines to stderr:

```text
cargo carapace 0.3.0 available (you have 0.2.0) · run: cargo carapace update
turn on automatic updates: cargo carapace update --auto on · skip: cargo carapace update --skip
```

The second line is dim. A version's notice appears at most once a day, never for a skipped
version, and never on stdout, so `cargo carapace schema > schema.json` stays clean.

The check runs at most once every 24 hours with a 4-second limit, and a failure is kept
for `update --check` to report instead of being printed. It is off when stdout or stderr is
not a terminal, when `CI` is set, under agent hosts, with `CARAPACE_NO_UPDATE_CHECK=1`, or
after `cargo carapace update --checks off`. See [PRIVACY.md](../PRIVACY.md) for exactly what
is sent and stored.

## Commands

| Command | What it does |
|---|---|
| `cargo carapace update` | Shows current → new and the start of the release notes, asks `Install 0.3.0 now? [Y/n]`, then runs `cargo install` for the release tag. Without a terminal it installs without asking. |
| `cargo carapace update --check` | Reports only. Exit 0: up to date. Exit 10: update available. Exit 1: the check failed, with the cause. |
| `cargo carapace update --skip` | No notice for this version until a newer one is released. |
| `cargo carapace update --auto on\|off\|status` | Install future updates automatically. Off by default. |
| `cargo carapace update --checks on\|off` | Turn the daily check on or off. |
| `cargo carapace update --rollback` | Reinstalls the version that ran before the last update and skips the one you left. |
| `cargo carapace update --clean` | Removes the state file and the log, for example before `cargo uninstall cargo-carapace`. |

After the first successful install, the CLI asks once: `Install future updates
automatically? [y/N]`. Either answer is remembered.

## Automatic updates

With `--auto on`, the check that finds a new release prints one line
(`… · installing in the background`) and starts `cargo install` as a detached, low-priority
process. The new version applies the next time you run the CLI; a running command is never
touched. If the install fails, the current version keeps working, the cause is shown by
`update --check` and `update --auto status`, and the normal notice returns the next day.
Background installs log to one file, `update.log`, kept under 256 KB and removed by
`--auto off`.

## How installs work

The CLI is installed with `cargo install`, so Cargo owns the binary. An update runs:

```sh
cargo install --git https://github.com/michael-berardi/carapace --tag v0.3.0 --locked cargo-carapace
```

with `--root` set to where the running binary was installed. The release tag fixes the
source, `--locked` makes Cargo build with the release's `Cargo.lock` and check every
crates.io dependency against its checksums, and Cargo replaces the binary only after a
successful build, so a failed build leaves the current binary in place. The build goes to
its own temporary folder, which is deleted whether the install succeeds or fails. A binary
that was not installed by `cargo install` (for example one built in a checkout) is never
replaced; `update` prints the command to run instead.

## Files

The check writes one file, `cargo-carapace/state.json`, in the platform cache folder
(`$XDG_CACHE_HOME`, `~/Library/Caches` or `~/.cache`), readable only by you. With
automatic updates on, `update.log` sits next to it. There is no rollback copy to clean up:
`--rollback` reinstalls the earlier tag through Cargo. To remove everything:

```sh
cargo carapace update --clean
cargo uninstall cargo-carapace
```

For tests, `CARAPACE_UPDATE_API` points the check at a local release record.
