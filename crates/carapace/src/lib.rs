//! # Carapace
//!
//! Write your application once as a Rust [`App`]; drive it from SwiftUI,
//! TypeScript (Tauri, Electron, Node), or anything that can call C.
//!
//! The core owns all state and logic. A shell renders the JSON state snapshot
//! and sends JSON actions. Typed Swift and TypeScript bindings are generated
//! from the core's JSON Schema, so shells stay thin and cannot drift.
//!
//! ```
//! use carapace::{App, Cx, NoEvent};
//! use schemars::JsonSchema;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Default, Serialize, Deserialize, JsonSchema)]
//! struct Config {}
//!
//! #[derive(Serialize, JsonSchema)]
//! struct State { count: i64 }
//!
//! #[derive(Clone, Serialize, Deserialize, JsonSchema)]
//! #[serde(tag = "type", rename_all = "camelCase")]
//! enum Action { Increment }
//!
//! struct Counter(i64);
//! impl App for Counter {
//!     type State = State;
//!     type Action = Action;
//!     type Event = NoEvent;
//!     type Config = Config;
//!     const NAME: &'static str = "Counter";
//!     fn init(_: Config, _: &mut Cx<Self>) -> Self { Counter(0) }
//!     fn update(&mut self, _: Action, _: &mut Cx<Self>) { self.0 += 1 }
//!     fn state(&self) -> State { State { count: self.0 } }
//! }
//!
//! let (mut engine, _) = carapace::Engine::<Counter>::start(Config {});
//! let out = engine.dispatch(Action::Increment);
//! assert_eq!(out.state.as_deref(), Some(r#"{"count":1}"#));
//! ```

mod engine;
pub mod ffi;
mod runtime;
mod schema;

pub use engine::{App, Cx, Effect, Engine, NoEvent, Outcome, Queries, Repeat, MAX_CHAIN};
pub use runtime::{query_json, Error, Handle, Notice, Runtime};
pub use schema::{fnv1a, schema, schema_hash, schema_with_queries, ABI_VERSION};

// Re-exported for the `export!` macro and for apps that derive their types.
pub use schemars;
pub use serde;
pub use serde_json;

/// Export an [`App`] over the C ABI. Put it once in the crate that builds the
/// `staticlib`/`cdylib`. One app per binary.
///
/// ```ignore
/// carapace::export!(MyApp);            // state, actions, events
/// carapace::export!(MyApp, queries);   // plus pure queries (`impl Queries for MyApp`)
/// ```
#[macro_export]
macro_rules! export {
    ($app:ty) => {
        $crate::__export_items!(
            $app,
            || $crate::schema::<$app>(),
            |_json, _len, error| unsafe {
                $crate::ffi::query_unsupported(<$app as $crate::App>::NAME, error)
            }
        );
    };
    ($app:ty, queries) => {
        $crate::__export_items!(
            $app,
            || $crate::schema_with_queries::<$app>(),
            |json, len, error| unsafe { $crate::ffi::query::<$app>(json, len, error) }
        );
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __export_items {
    ($app:ty, $schema:expr, $query:expr) => {
        #[no_mangle]
        pub extern "C" fn carapace_abi_version() -> u32 {
            $crate::ffi::abi_version()
        }
        #[no_mangle]
        pub extern "C" fn carapace_schema_hash() -> u64 {
            $crate::ffi::hash($schema)
        }
        #[no_mangle]
        pub extern "C" fn carapace_schema() -> *mut ::std::ffi::c_char {
            $crate::ffi::schema_json($schema)
        }
        /// # Safety
        /// `config` is null or NUL-terminated; `error` is null or a valid out-pointer.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_start(
            config: *const ::std::ffi::c_char,
            error: *mut *mut ::std::ffi::c_char,
        ) -> *mut $crate::ffi::CarapaceHandle {
            $crate::ffi::start::<$app>(config, error)
        }
        /// # Safety
        /// `handle` from `carapace_start`; `json` points to `len` bytes.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_dispatch(
            handle: *mut $crate::ffi::CarapaceHandle,
            json: *const u8,
            len: usize,
        ) -> *mut ::std::ffi::c_char {
            $crate::ffi::dispatch(handle, json, len)
        }
        /// # Safety
        /// `handle` from `carapace_start`; `json` points to `len` bytes. Not from a callback.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_dispatch_wait(
            handle: *mut $crate::ffi::CarapaceHandle,
            json: *const u8,
            len: usize,
        ) -> *mut ::std::ffi::c_char {
            $crate::ffi::dispatch_wait(handle, json, len)
        }
        /// # Safety
        /// `json` points to `len` bytes; `error` is null or a valid out-pointer.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_query(
            json: *const u8,
            len: usize,
            error: *mut *mut ::std::ffi::c_char,
        ) -> *mut ::std::ffi::c_char {
            let run: fn(*const u8, usize, *mut *mut ::std::ffi::c_char) -> *mut ::std::ffi::c_char =
                $query;
            run(json, len, error)
        }
        /// # Safety
        /// `handle` from `carapace_start`.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_state(
            handle: *mut $crate::ffi::CarapaceHandle,
        ) -> *mut ::std::ffi::c_char {
            $crate::ffi::state(handle)
        }
        /// # Safety
        /// `handle` from `carapace_start`; `user` outlives the subscription.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_subscribe(
            handle: *mut $crate::ffi::CarapaceHandle,
            callback: $crate::ffi::Callback,
            user: *mut ::std::ffi::c_void,
        ) -> u64 {
            $crate::ffi::subscribe(handle, callback, user)
        }
        /// # Safety
        /// `handle` from `carapace_start`.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_unsubscribe(
            handle: *mut $crate::ffi::CarapaceHandle,
            id: u64,
        ) {
            $crate::ffi::unsubscribe(handle, id)
        }
        /// # Safety
        /// `handle` from `carapace_start`; invalid afterwards.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_stop(handle: *mut $crate::ffi::CarapaceHandle) {
            $crate::ffi::stop(handle)
        }
        /// # Safety
        /// `s` is an owned string returned by a carapace function.
        #[no_mangle]
        pub unsafe extern "C" fn carapace_string_free(s: *mut ::std::ffi::c_char) {
            $crate::ffi::string_free(s)
        }
    };
}
