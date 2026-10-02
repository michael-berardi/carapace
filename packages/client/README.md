# @carapace/client

TypeScript client for [Carapace](https://github.com/michael-berardi/carapace) cores: a typed,
observable store over Tauri, the C ABI from Node and Electron, or any transport.

Each release attaches the package as a tarball (the npm registry listing is pending):

```sh
npm i https://github.com/michael-berardi/carapace/releases/download/v0.1.0/carapace-client-0.1.0.tgz
```

```ts
import { Store } from "@carapace/client";
import { tauriTransport } from "@carapace/client/tauri";   // Tauri 2
import { nodeTransport } from "@carapace/client/node";     // Node, Electron main (needs koffi)
import { useCarapace } from "@carapace/client/react";      // React 18+
import { actions, appName, schemaHash, type Types } from "./generated/MyApp";

const store = await Store.connect<Types>(tauriTransport(), { schemaHash, appName });
await store.dispatch(actions.increment());
store.state;                          // current state, a new object on every change
store.onEvent((e) => { /* platform requests from the core */ });
await store.query({ type: "describe", value: 3 });   // pure query, when the core exports them
store.faults;                         // core panics and rejected actions
```

`Store.connect` rejects with `StaleBindingsError` when the generated file does not match the core.
Generate the types with `cargo carapace gen ts`. See the
[getting started guide](https://github.com/michael-berardi/carapace/blob/main/docs/getting-started.md).
