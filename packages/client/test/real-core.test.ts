import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import { Store } from "../src/index.js";
import { nodeTransport } from "../src/node.js";

// Runs the real Rust core (examples/counter) through the real C ABI from Node.
const root = join(dirname(fileURLToPath(import.meta.url)), "../../..");
const ext = process.platform === "darwin" ? "dylib" : process.platform === "win32" ? "dll" : "so";
const lib = join(root, "target/debug", (process.platform === "win32" ? "" : "lib") + `counter_core.${ext}`);
type CounterTypes = import("../../../examples/counter/web/Counter.js").Types;

beforeAll(() => {
  execFileSync("cargo", ["build", "-q", "-p", "counter-core", "-p", "cargo-carapace"], { cwd: root, stdio: "inherit" });
  expect(existsSync(lib)).toBe(true);
});

async function until(ok: () => boolean, what: string) {
  for (let i = 0; i < 300; i++) {
    if (ok()) return;
    await new Promise((r) => setTimeout(r, 10));
  }
  throw new Error(`timed out waiting for ${what}`);
}

describe("real Rust core from Node", () => {
  it("drives state, timers, background work and events", async () => {
    const { schemaHash, actions } = await import("../../../examples/counter/web/Counter.js");
    const transport = await nodeTransport(lib, { config: { start: 10 } });
    const store = await Store.connect<CounterTypes>(transport, { schemaHash, appName: "Counter" });
    try {
      expect(store.state.count).toBe(10);
      const titles: string[] = [];
      store.onEvent((e) => titles.push(e.title));
      expect(titles).toEqual(["Counter"]);

      await store.dispatch(actions.setStep({ step: 5 }));
      await store.dispatch(actions.increment());
      await until(() => store.state.count === 15, "count 15");

      await store.dispatch(actions.startTicking());
      await until(() => store.state.count >= 18, "ticks");
      await store.dispatch(actions.stopTicking());
      await store.dispatch(actions.fetch());
      await until(() => store.state.count === 100 && store.state.mode === "idle", "fetch");
      await until(() => titles.includes("Fetched"), "fetched event");

      await store.dispatch(actions.rename({ label: "Groceries" }));
      await until(() => store.state.label === "Groceries", "label");
    } finally {
      await store.close();
    }
  });

  it("delivers newtype event variants with the wrapped struct's fields beside the tag", async () => {
    const { schemaHash, actions } = await import("../../../examples/counter/web/Counter.js");
    const store = await Store.connect<CounterTypes>(await nodeTransport(lib, { config: { start: 7 } }), { schemaHash });
    try {
      const seen: unknown[] = [];
      store.onEvent((e) => e.type === "summary" && seen.push(e));
      await store.dispatch(actions.increment());
      await store.dispatch(actions.summarize());
      await until(() => seen.length > 0, "summary event");
      expect(seen[0]).toEqual({ type: "summary", count: 8, entries: 1 });
    } finally {
      await store.close();
    }
  });

  it("runs pure queries without touching state", async () => {
    const { schemaHash } = await import("../../../examples/counter/web/Counter.js");
    const store = await Store.connect<CounterTypes>(await nodeTransport(lib), { schemaHash });
    try {
      expect(await store.query({ type: "describe", value: -3 })).toEqual({ text: "negative, odd" });
      expect(store.state.count).toBe(0);
      await expect(store.query({ type: "nope" } as never)).rejects.toThrow(/Counter: cannot decode query/);
    } finally {
      await store.close();
    }
  });

  it("closes cleanly while a timer keeps notifying (no deadlock with koffi callbacks)", async () => {
    const { schemaHash, actions } = await import("../../../examples/counter/web/Counter.js");
    for (let i = 0; i < 15; i++) {
      const store = await Store.connect<CounterTypes>(await nodeTransport(lib), { schemaHash });
      store.onEvent(() => {});
      await store.dispatch(actions.startTicking());
      for (let k = 0; k < 10; k++) await store.dispatch(actions.increment());
      await new Promise((r) => setTimeout(r, i % 3 === 0 ? 0 : 30));
      await store.close();
    }
  }, 30_000);

  it("names the problem when the core cannot decode an action", async () => {
    const { schemaHash } = await import("../../../examples/counter/web/Counter.js");
    const transport = await nodeTransport(lib);
    const store = await Store.connect<CounterTypes>(transport, { schemaHash });
    try {
      await expect(store.dispatch({ type: "nope" } as never)).rejects.toThrow(/Counter: cannot decode action/);
      expect(store.faults[0]).toContain("could not send");
    } finally {
      await store.close();
    }
  });

  it("reports a stale schema hash", async () => {
    const transport = await nodeTransport(lib);
    await expect(Store.connect<CounterTypes>(transport, { schemaHash: "0x1", appName: "Counter" })).rejects.toThrow(/stale/);
    await transport.close?.();
  });
});
