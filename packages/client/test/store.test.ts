import { describe, expect, it } from "vitest";
import { Store, StaleBindingsError, type Notice, type Transport, type Types } from "../src/index.js";

interface T extends Types {
  state: { n: number };
  action: { type: "add"; by: number };
  event: { type: "ping"; text: string };
  config: Record<string, never>;
}

class Fake implements Transport {
  n = 0;
  sent: string[] = [];
  listener?: (n: Notice) => void;
  backlog: Notice[] = [{ kind: "event", json: '{"type":"ping","text":"start"}' }];
  failWith?: string;
  constructor(public hash = "0xabc") {}
  schemaHash = async () => this.hash;
  query = async (q: string) => JSON.stringify({ echo: JSON.parse(q) });
  snapshot = async () => JSON.stringify({ n: this.n });
  async dispatch(json: string) {
    if (this.failWith) throw new Error(this.failWith);
    this.sent.push(json);
  }
  async subscribe(on: (n: Notice) => void) {
    this.listener = on;
    this.backlog.splice(0).forEach(on);
    return () => (this.listener = undefined);
  }
  push(n: Notice) {
    this.listener?.(n);
  }
}

const connect = (t: Fake) => Store.connect<T>(t, { schemaHash: "0xABC", appName: "Fake" });

describe("Store", () => {
  it("starts from the snapshot and follows state notices with a new object each time", async () => {
    const t = new Fake();
    t.n = 3;
    const s = await connect(t);
    expect(s.state).toEqual({ n: 3 });
    const before = s.state;
    let renders = 0;
    s.subscribe(() => renders++);
    t.push({ kind: "state", json: '{"n":4}' });
    expect(s.state).toEqual({ n: 4 });
    expect(s.state).not.toBe(before);
    expect(renders).toBe(1);
  });

  it("serialises actions as the tagged JSON the core expects", async () => {
    const t = new Fake();
    const s = await connect(t);
    await s.dispatch({ type: "add", by: 2 });
    expect(t.sent).toEqual(['{"type":"add","by":2}']);
  });

  it("holds start-up events until a handler exists, in order, and never drops them", async () => {
    const t = new Fake();
    const s = await connect(t);
    t.push({ kind: "event", json: '{"type":"ping","text":"two"}' });
    const got: string[] = [];
    s.onEvent((e) => got.push(e.text));
    expect(got).toEqual(["start", "two"]);
    t.push({ kind: "event", json: '{"type":"ping","text":"three"}' });
    expect(got).toEqual(["start", "two", "three"]);
  });

  it("rejects stale bindings by name", async () => {
    await expect(connect(new Fake("0xdef"))).rejects.toBeInstanceOf(StaleBindingsError);
    await expect(connect(new Fake("0xdef"))).rejects.toThrow(/Fake bindings are stale/);
  });

  it("records faults from the core and from failed dispatches, and rethrows", async () => {
    const t = new Fake();
    const s = await connect(t);
    t.push({ kind: "fault", text: "Fake: update panicked: boom" });
    t.failWith = "Fake: cannot decode action";
    await expect(s.dispatch({ type: "add", by: 1 })).rejects.toThrow("cannot decode");
    expect(s.faults).toHaveLength(2);
    expect(s.faults[1]).toContain("could not send");
  });

  it("keeps only the newest 50 faults", async () => {
    const t = new Fake();
    const s = await connect(t);
    for (let i = 0; i < 80; i++) t.push({ kind: "fault", text: `f${i}` });
    expect(s.faults).toHaveLength(50);
    expect(s.faults.at(-1)).toBe("f79");
  });

  it("applies notices that arrive while connecting", async () => {
    const t = new Fake();
    const original = t.subscribe.bind(t);
    t.subscribe = async (on) => {
      const un = await original(on);
      on({ kind: "state", json: '{"n":9}' });
      return un;
    };
    expect((await connect(t)).state).toEqual({ n: 9 });
  });
});
