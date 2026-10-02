# Security policy

Carapace runs your own Rust code inside your own app. It opens no sockets and reads no files
of its own. The surfaces worth attention are the C ABI (memory ownership, lengths, threading),
the JSON decoding of actions, queries and config, and the Tauri command allowlist.

## Reporting a vulnerability

Please report privately through GitHub's
[security advisories](https://github.com/michael-berardi/carapace/security/advisories/new)
rather than a public issue. Include the version, platform and the smallest reproduction you can
manage. We aim to acknowledge reports within a week.

Particularly interesting: any input that crosses the ABI and causes memory unsafety, a panic that
unwinds into a host, a callback that runs after `carapace_stop` returns, or a Tauri command that
works without the `carapace:default` permission.
