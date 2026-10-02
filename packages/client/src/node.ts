import type { Notice, Transport } from "./index.js";

/**
 * Drives a Carapace core built as a `cdylib` from Node or an Electron main process,
 * through the C ABI, using koffi (an optional peer dependency).
 *
 * ```ts
 * const store = await Store.connect<Types>(await nodeTransport("./libmycore.dylib"), { schemaHash });
 * ```
 */
let callbackProto: import("koffi").IKoffiCType | undefined;

export async function nodeTransport(libraryPath: string, options: { config?: unknown } = {}): Promise<Transport> {
  let koffi: typeof import("koffi");
  try {
    const mod = (await import("koffi")) as unknown as { default?: typeof import("koffi") };
    koffi = mod.default ?? (mod as unknown as typeof import("koffi"));
  } catch (e) {
    throw new Error(`@carapace/client/node needs the "koffi" package (npm i koffi): ${e instanceof Error ? e.message : e}`);
  }
  const lib = koffi.load(libraryPath);
  // koffi type names are process-global, so define the callback type once.
  callbackProto ??= koffi.proto("void CarapaceCallback(void *user, uint32_t kind, void *data, size_t len)");
  const Callback = callbackProto;

  const abiVersion = lib.func("uint32_t carapace_abi_version()");
  const schemaHash = lib.func("uint64_t carapace_schema_hash()");
  const start = lib.func("void *carapace_start(const char *config, _Out_ void **error)");
  const dispatch = lib.func("void *carapace_dispatch(void *h, const char *json, size_t len)");
  const query = lib.func("void *carapace_query(const char *json, size_t len, _Out_ void **error)");
  const state = lib.func("void *carapace_state(void *h)");
  const subscribe = lib.func("uint64_t carapace_subscribe(void *h, CarapaceCallback *cb, void *user)");
  const unsubscribe = lib.func("void carapace_unsubscribe(void *h, uint64_t id)");
  const stop = lib.func("void carapace_stop(void *h)");
  const stringFree = lib.func("void carapace_string_free(void *s)");

  if (abiVersion() !== 1) throw new Error(`Carapace ABI mismatch: this client speaks 1, ${libraryPath} speaks ${abiVersion()}`);

  const takeString = (p: unknown): string | null => {
    if (!p) return null;
    const text = koffi.decode(p, "char", -1) as unknown as string;
    stringFree(p);
    return text;
  };

  const err: unknown[] = [null];
  const configText = options.config === undefined ? null : JSON.stringify(options.config);
  const handle = start(configText, err);
  if (!handle) throw new Error(takeString(err[0]) ?? "the core failed to start without an error message");

  // koffi delivers core-thread callbacks on the JS thread. A synchronous call that waits for the
  // core thread (unsubscribe, stop) would block the JS thread while a callback waits for it, so
  // those two run on koffi's worker pool and the JS thread stays free to serve callbacks.
  const inWorker = <T = void>(fn: { async: (...args: unknown[]) => void }, ...args: unknown[]) =>
    new Promise<T>((resolve, reject) => fn.async(...args, (err: unknown, result: T) => (err ? reject(err) : resolve(result))));

  const callbacks = new Map<number, unknown>();
  const pending = new Set<Promise<void>>();
  let stopped = false;

  const live = () => {
    if (stopped) throw new Error("the core has been stopped");
  };

  return {
    schemaHash: async () => "0x" + (schemaHash() as bigint).toString(16).padStart(16, "0"),
    snapshot: async () => {
      live();
      const s = takeString(state(handle));
      if (s === null) throw new Error("the core returned no state");
      return s;
    },
    query: async (json) => {
      live();
      const e: unknown[] = [null];
      const answer = takeString(query(json, Buffer.byteLength(json, "utf8"), e));
      if (answer === null) throw new Error(takeString(e[0]) ?? "the core failed a query without an error message");
      return answer;
    },
    dispatch: async (json) => {
      live();
      const bytes = Buffer.byteLength(json, "utf8");
      const message = takeString(dispatch(handle, json, bytes));
      if (message) throw new Error(message);
    },
    subscribe: async (onNotice: (n: Notice) => void) => {
      live();
      const cb = koffi.register((_user: unknown, kind: number, data: unknown, len: number) => {
        const text = Buffer.from(koffi.decode(data, "uint8_t", len) as Uint8Array).toString("utf8");
        onNotice(kind === 0 ? { kind: "state", json: text } : kind === 1 ? { kind: "event", json: text } : { kind: "fault", text });
      }, koffi.pointer(Callback));
      // Subscribing waits on the delivery lock, so it too must not block the JS thread. It is tracked
      // so `close` waits for it instead of freeing the handle under it.
      const subscribing = inWorker<bigint>(subscribe, handle, cb, null);
      const tracked = subscribing.then(
        () => {},
        () => {},
      );
      const settle = tracked.finally(() => pending.delete(settle));
      pending.add(settle);
      const id = Number(await subscribing);
      if (stopped) {
        koffi.unregister(cb as never);
        return () => {};
      }
      callbacks.set(id, cb);
      return () => {
        if (stopped) return;
        // After unsubscribe returns, no callback is running or will start: safe to unregister.
        const done = inWorker(unsubscribe, handle, id)
          .then(() => {
            koffi.unregister(callbacks.get(id) as never);
            callbacks.delete(id);
          })
          .catch((e) => console.error("carapace: unsubscribe failed:", e))
          .finally(() => pending.delete(done));
        pending.add(done);
      };
    },
    close: async () => {
      if (stopped) return;
      stopped = true;
      await Promise.all(pending);
      await inWorker(stop, handle);
      // The core thread is gone: no callback can run, so release what is left.
      for (const cb of callbacks.values()) koffi.unregister(cb as never);
      callbacks.clear();
    },
  };
}
