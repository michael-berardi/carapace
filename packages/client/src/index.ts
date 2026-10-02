/** The four types a generated Carapace binding file exports as `Types`. */
export interface Types {
  state: unknown;
  action: unknown;
  event: unknown;
  config: unknown;
  /** `never` when the core exports no queries. */
  query?: unknown;
  answer?: unknown;
}

/** What a running core tells its subscribers. Payloads are JSON text. */
export type Notice =
  | { kind: "state"; json: string }
  | { kind: "event"; json: string }
  | { kind: "fault"; text: string };

/** How a store talks to a core: Tauri commands, the C ABI from Node, or a test double. */
export interface Transport {
  /** Hex string such as "0x8da78566d74c3ac2". */
  schemaHash(): Promise<string>;
  snapshot(): Promise<string>;
  /** A pure, stateless query (JSON in, JSON out). */
  query(queryJson: string): Promise<string>;
  /** Rejects with the core's own message when the action cannot be decoded. */
  dispatch(actionJson: string): Promise<void>;
  /** Events emitted before the first subscription are replayed to it. */
  subscribe(onNotice: (notice: Notice) => void): Promise<() => void>;
  close?(): void | Promise<void>;
}

export class StaleBindingsError extends Error {
  constructor(app: string, expected: string, found: string) {
    super(
      `The generated ${app} bindings are stale: they were generated from schema ${expected} but the core has schema ${found}. ` +
        "Run `cargo carapace gen ts` and rebuild.",
    );
    this.name = "StaleBindingsError";
  }
}

export interface ConnectOptions {
  /** `schemaHash` exported by the generated bindings. */
  schemaHash: string;
  /** `appName` exported by the generated bindings; used in error messages. */
  appName?: string;
}

const MAX_FAULTS = 50;

/**
 * A live view of a core's state. Compatible with React's `useSyncExternalStore`
 * (`subscribe` / `getSnapshot`), and usable without React.
 */
export class Store<T extends Types> {
  private current: T["state"];
  private listeners = new Set<() => void>();
  private handlers = new Set<(event: T["event"]) => void>();
  private held: T["event"][] = [];
  private _faults: readonly string[] = [];
  private unsubscribe: () => void = () => {};
  private sawState = false;

  private constructor(
    private readonly transport: Transport,
    initial: T["state"],
  ) {
    this.current = initial;
  }

  static async connect<T extends Types>(transport: Transport, options: ConnectOptions): Promise<Store<T>> {
    const found = (await transport.schemaHash()).toLowerCase();
    if (found !== options.schemaHash.toLowerCase()) {
      throw new StaleBindingsError(options.appName ?? "Carapace", options.schemaHash, found);
    }
    // Subscribe first so no update falls between subscribing and reading the snapshot.
    const pending: Notice[] = [];
    let store: Store<T> | undefined;
    const unsubscribe = await transport.subscribe((n) => (store ? store.receive(n) : pending.push(n)));
    const initial = JSON.parse(await transport.snapshot()) as T["state"];
    store = new Store<T>(transport, initial);
    store.unsubscribe = unsubscribe;
    for (const n of pending) store.receive(n);
    return store;
  }

  /** The current state. A new object on every change, so identity comparison works. */
  get state(): T["state"] {
    return this.current;
  }

  /** Messages from the core and failed dispatches, newest last (max 50). */
  get faults(): readonly string[] {
    return this._faults;
  }

  /** `useSyncExternalStore` contract. */
  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): T["state"] => this.current;

  /**
   * Send an action. Resolves when the core has queued it (not when it has been applied);
   * the new state arrives through `subscribe`. Rejections are also recorded in `faults`.
   */
  async dispatch(action: T["action"]): Promise<void> {
    try {
      await this.transport.dispatch(JSON.stringify(action));
    } catch (e) {
      this.fault(`could not send ${JSON.stringify(action)}: ${e instanceof Error ? e.message : String(e)}`);
      throw e;
    }
  }

  /**
   * Run one of the core's pure queries (colour math, parsing, formatting). Stateless: it
   * does not touch the core's state.
   */
  async query(query: NonNullable<T["query"]>): Promise<NonNullable<T["answer"]>> {
    return JSON.parse(await this.transport.query(JSON.stringify(query))) as NonNullable<T["answer"]>;
  }

  /**
   * Handle platform requests from the core. Events that arrived before the first handler
   * are delivered to it, in order, so none from start-up is lost.
   */
  onEvent(handler: (event: T["event"]) => void): () => void {
    this.handlers.add(handler);
    const held = this.held;
    this.held = [];
    for (const e of held) handler(e);
    return () => this.handlers.delete(handler);
  }

  async close(): Promise<void> {
    this.unsubscribe();
    await this.transport.close?.();
  }

  private receive(n: Notice): void {
    switch (n.kind) {
      case "state":
        this.sawState = true;
        this.current = JSON.parse(n.json) as T["state"];
        this.emit();
        break;
      case "event": {
        const event = JSON.parse(n.json) as T["event"];
        if (this.handlers.size === 0) this.held.push(event);
        else for (const h of this.handlers) h(event);
        break;
      }
      case "fault":
        this.fault(n.text);
        break;
    }
  }

  private fault(text: string): void {
    this._faults = [...this._faults, text].slice(-MAX_FAULTS);
    this.emit();
  }

  private emit(): void {
    for (const l of [...this.listeners]) l();
  }
}
