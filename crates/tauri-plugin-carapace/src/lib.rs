//! Host a Carapace core inside a Tauri 2 app.
//!
//! ```ignore
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_carapace::plugin::<MyApp, _>(Default::default()))
//!     .run(tauri::generate_context!())
//! ```
//!
//! Add `"carapace:default"` to your capability file, then use `@carapace/client`
//! in the webview. State goes to the webview as the `carapace://state` event,
//! platform requests as `carapace://event`, faults as `carapace://fault`.

use carapace::{App, Notice, Queries, Runtime as Core};
use std::sync::Mutex;
use tauri::plugin::{Builder, TauriPlugin};

use tauri::{AppHandle, Emitter, Manager, State};

type Forward = Box<dyn Fn(Notice<'_>) + Send + Sync>;
type DispatchFn = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Type-erased view of the running core, kept in Tauri state.
struct Hosted {
    dispatch: DispatchFn,
    snapshot: Box<dyn Fn() -> String + Send + Sync>,
    subscribe: Box<dyn Fn(Forward) -> u64 + Send + Sync>,
    unsubscribe: Box<dyn Fn(u64) + Send + Sync>,
    attached: Mutex<Option<u64>>,
    query: QueryFn,
    schema_hash: u64,
    /// Dropping this stops the core and joins its thread.
    _core: Box<dyn std::any::Any + Send + Sync>,
}

#[tauri::command]
fn dispatch(hosted: State<'_, Hosted>, action: String) -> Result<(), String> {
    (hosted.dispatch)(&action)
}

#[tauri::command]
fn state(hosted: State<'_, Hosted>) -> String {
    (hosted.snapshot)()
}

/// Run one of the core's pure queries (JSON in, JSON out).
#[tauri::command]
fn query(hosted: State<'_, Hosted>, query: String) -> Result<String, String> {
    (hosted.query)(&query)
}

/// Hex string, so JavaScript never rounds a u64.
#[tauri::command]
fn schema_hash(hosted: State<'_, Hosted>) -> String {
    format!("0x{:016x}", hosted.schema_hash)
}

/// Start forwarding state, events and faults to the webview. Events the core emitted
/// before the first attach (including from `init`) are replayed. Calling it again
/// (page reload) replaces the previous forwarder.
#[tauri::command]
fn attach<R: tauri::Runtime>(app: AppHandle<R>, hosted: State<'_, Hosted>) {
    let mut slot = hosted.attached.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(old) = slot.take() {
        (hosted.unsubscribe)(old);
    }
    let id = (hosted.subscribe)(Box::new(move |notice| {
        let (name, payload) = match notice {
            Notice::State(s) => ("carapace://state", s),
            Notice::Event(s) => ("carapace://event", s),
            Notice::Fault(s) => ("carapace://fault", s),
        };
        if let Err(e) = app.emit(name, payload) {
            eprintln!("tauri-plugin-carapace: cannot emit {name}: {e}");
        }
    }));
    *slot = Some(id);
}

/// The plugin. `A::Config` is passed to the core at start-up.
pub fn plugin<A: App, R: tauri::Runtime>(config: A::Config) -> TauriPlugin<R> {
    build::<A, R>(
        config,
        || carapace::schema::<A>(),
        Box::new(|_| {
            Err(format!(
                "{}: this core exports no queries; use plugin_with_queries",
                A::NAME
            ))
        }),
    )
}

/// Like [`plugin`], for cores that implement [`Queries`].
pub fn plugin_with_queries<A: Queries, R: tauri::Runtime>(config: A::Config) -> TauriPlugin<R> {
    build::<A, R>(
        config,
        || carapace::schema_with_queries::<A>(),
        Box::new(carapace::query_json::<A>),
    )
}

type QueryFn = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

fn build<A: App, R: tauri::Runtime>(
    config: A::Config,
    schema: fn() -> serde_json::Value,
    query_fn: QueryFn,
) -> TauriPlugin<R> {
    let config = Mutex::new(Some(config));
    let query_slot = Mutex::new(Some(query_fn));
    Builder::new("carapace")
        .invoke_handler(tauri::generate_handler![
            dispatch,
            state,
            schema_hash,
            attach,
            query
        ])
        .setup(move |app, _api| {
            let config = config
                .lock()
                .map_err(|_| "tauri-plugin-carapace: plugin config lock poisoned")?
                .take()
                .ok_or("tauri-plugin-carapace: the plugin was set up twice")?;
            let query_fn = query_slot
                .lock()
                .map_err(|_| "tauri-plugin-carapace: plugin query lock poisoned")?
                .take()
                .ok_or("tauri-plugin-carapace: the plugin was set up twice")?;
            let core = Core::<A>::start(config);
            let h = core.handle();
            let (h_dispatch, h_snapshot, h_sub, h_unsub) = (h.clone(), h.clone(), h.clone(), h);
            app.manage(Hosted {
                dispatch: Box::new(move |json| {
                    h_dispatch.dispatch_json(json).map_err(|e| e.to_string())
                }),
                snapshot: Box::new(move || h_snapshot.snapshot().to_string()),
                subscribe: Box::new(move |f| h_sub.subscribe(f)),
                unsubscribe: Box::new(move |id| h_unsub.unsubscribe(id)),
                attached: Mutex::new(None),
                query: query_fn,
                schema_hash: carapace::schema_hash(&schema()),
                _core: Box::new(core),
            });
            Ok(())
        })
        .build()
}
