import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Notice, Transport } from "./index.js";

/** Talks to the core hosted by the `carapace-tauri` plugin. */
export function tauriTransport(): Transport {
  return {
    schemaHash: () => invoke<string>("plugin:carapace|schema_hash"),
    snapshot: () => invoke<string>("plugin:carapace|state"),
    query: (query) => invoke<string>("plugin:carapace|query", { query }),
    dispatch: (action) => invoke<void>("plugin:carapace|dispatch", { action }),
    async subscribe(onNotice: (n: Notice) => void) {
      const un: UnlistenFn[] = await Promise.all([
        listen<string>("carapace://state", (e) => onNotice({ kind: "state", json: e.payload })),
        listen<string>("carapace://event", (e) => onNotice({ kind: "event", json: e.payload })),
        listen<string>("carapace://fault", (e) => onNotice({ kind: "fault", text: e.payload })),
      ]);
      // Listeners are in place: now ask Rust to start forwarding (and replay start-up events).
      await invoke<void>("plugin:carapace|attach");
      return () => un.forEach((f) => f());
    },
  };
}
