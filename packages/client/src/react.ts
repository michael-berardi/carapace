import { useEffect, useSyncExternalStore } from "react";
import type { Store, Types } from "./index.js";

/** Subscribe a component to a store's state. Re-renders only when the state changes. */
export function useCarapace<T extends Types>(store: Store<T>): T["state"] {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

/** Handle the core's platform requests for as long as the component is mounted. */
export function useCarapaceEvents<T extends Types>(store: Store<T>, handler: (event: T["event"]) => void): void {
  useEffect(() => store.onEvent(handler), [store, handler]);
}

/** Faults reported by the core or by failed dispatches. */
export function useCarapaceFaults<T extends Types>(store: Store<T>): readonly string[] {
  return useSyncExternalStore(store.subscribe, () => store.faults, () => store.faults);
}
