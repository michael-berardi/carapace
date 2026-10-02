import { Store } from "@carapace/client";
import { useCarapace, useCarapaceEvents, useCarapaceFaults } from "@carapace/client/react";
import { tauriTransport } from "@carapace/client/tauri";
import { StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { actions, appName, schemaHash, type Event, type Types } from "./generated/Counter";

type CounterStore = Store<Types>;

function App({ store }: { store: CounterStore }) {
  const s = useCarapace(store);
  const faults = useCarapaceFaults(store);
  const [banner, setBanner] = useState<string | null>(null);
  const onEvent = useCallback((e: Event) => {
    if (e.type === "notify") {
      setBanner(`${e.title}: ${e.body}`);
      setTimeout(() => setBanner(null), 2500);
    }
  }, []);
  useCarapaceEvents(store, onEvent);

  const peak = Math.max(1, ...s.history.map(Math.abs));
  return (
    <main>
      <h1>{s.label ?? "Counter"}</h1>
      <div className="count" aria-live="polite">{s.count}</div>
      <div className="row">
        <button className="round" aria-label="Decrement" onClick={() => store.dispatch(actions.decrement())}>−</button>
        <button className="round" aria-label="Increment" onClick={() => store.dispatch(actions.increment())}>+</button>
      </div>
      <label className="row">
        Step {s.step}
        <input type="range" min={1} max={100} value={s.step}
          onChange={(e) => store.dispatch(actions.setStep({ step: Number(e.target.value) }))} />
      </label>
      <div className="row">
        <button onClick={() => store.dispatch(s.mode === "ticking" ? actions.stopTicking() : actions.startTicking())}>
          {s.mode === "ticking" ? "Stop ticking" : "Tick every 250 ms"}
        </button>
        <button disabled={s.mode === "fetching"} onClick={() => store.dispatch(actions.fetch())}>Fetch</button>
        <button className="danger" onClick={() => store.dispatch(actions.reset())}>Reset</button>
      </div>
      <div className="bars" role="img" aria-label="Recent values">
        {s.history.map((v, i) => (
          <i key={i} className={v < 0 ? "neg" : ""} style={{ height: Math.max(4, (44 * Math.abs(v)) / peak) }} />
        ))}
      </div>
      {banner && <div className="banner">🔔 {banner}</div>}
      {faults.length > 0 && <div className="fault">{faults[faults.length - 1]}</div>}
    </main>
  );
}

const root = createRoot(document.getElementById("root")!);
Store.connect<Types>(tauriTransport(), { schemaHash, appName }).then(
  (store) => root.render(<StrictMode><App store={store} /></StrictMode>),
  (e) => { document.body.textContent = `Counter core did not start: ${e instanceof Error ? e.message : e}`; },
);
