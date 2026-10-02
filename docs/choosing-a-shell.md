# Choosing a shell

The core does not care which shell drives it, so the choice is about the app's
users and the team. How each option fits Carapace today:

| Shell | Strengths | Costs | With Carapace |
|---|---|---|---|
| **SwiftUI** | Looks and behaves like a Mac app: menu bar, Spotlight and Shortcuts, widgets, accessibility, Apple Silicon. Small and fast. | Apple platforms only. | First-class. `CarapaceKit` plus generated types. Tested. |
| **Tauri (webview)** | One UI codebase for Windows, Linux and macOS. Web talent and ecosystem. Far lighter than Electron. | A webview's memory and startup; the UI feels slightly off beside native apps. | `tauri-plugin-carapace` plus `@carapace/client`. Tested on macOS. |
| **Electron** | Most mature cross-platform option. | Heaviest on memory, battery and startup. | The Node transport loads the core through the C ABI in the main process. Tested in Node, not yet inside Electron. |
| **Flutter** | Consistent custom-drawn UI everywhere, good performance. | No native controls. | Call the C ABI through `dart:ffi`. No generator yet; the wire format is plain JSON. |
| **React Native (macOS, Windows)** | Reuse React skills. | macOS and Windows support trails mobile. | The Node/C ABI approach applies through a native module. Not built. |
| **Qt / C++** | Heavy custom needs, mature desktop toolkit. | Dated look, licensing. | `carapace.h` is a C header. Not built. |
| **Catalyst, SwiftUI multi-platform** | Share code across Mac, iPad, iPhone. | Apple only. | The same `CarapaceKit` store. the iOS xcframework slices build with `--ios`; `CarapaceKit` has not been compiled for iOS yet. |

A common split, and the one Good Night uses: native SwiftUI on macOS, a Tauri shell on Windows,
and one Rust engine under both. The macOS app stays a real Mac app; Windows gets a shell that
costs a webview and nothing else, and the behaviour cannot diverge because there is one
implementation.

## When Carapace is not the right tool

- The app is a thin client over a server. There is no core to share.
- The UI is the product and logic is trivial. A single-platform UI toolkit alone is simpler.
- You need to share Rust *types* with Swift or Kotlin at fine granularity (many small
  synchronous calls). Use a function-level binding generator such as UniFFI instead; Carapace
  is deliberately coarse-grained: actions in, snapshots out.
