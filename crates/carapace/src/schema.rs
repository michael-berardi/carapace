//! The machine-readable contract between a core and its shells.

use schemars::generate::SchemaSettings;
use serde_json::{json, Value};

use crate::engine::{App, Queries};

/// Bumped when the C ABI or the wire format changes incompatibly.
pub const ABI_VERSION: u32 = 1;

/// JSON Schema bundle describing an app's config, state, actions and events.
pub fn schema<A: App>() -> Value {
    bundle::<A>(|_| None)
}

/// Like [`schema`], plus the app's pure query and answer types.
pub fn schema_with_queries<A: Queries>() -> Value {
    bundle::<A>(|g| {
        Some((
            g.subschema_for::<A::Query>(),
            g.subschema_for::<A::Answer>(),
        ))
    })
}

type QuerySchemas = Option<(schemars::Schema, schemars::Schema)>;

fn bundle<A: App>(queries: impl FnOnce(&mut schemars::SchemaGenerator) -> QuerySchemas) -> Value {
    let mut generator = SchemaSettings::draft07().into_generator();
    let config = generator.subschema_for::<A::Config>();
    let state = generator.subschema_for::<A::State>();
    let action = generator.subschema_for::<A::Action>();
    let event = generator.subschema_for::<A::Event>();
    let queries = queries(&mut generator);
    let definitions = Value::Object(generator.take_definitions(true));
    let mut out = json!({
        "carapace": ABI_VERSION,
        "name": A::NAME,
        "config": config,
        "state": state,
        "action": action,
        "event": event,
        "definitions": definitions,
    });
    if let Some((query, answer)) = queries {
        out["query"] = json!(query);
        out["answer"] = json!(answer);
    }
    out
}

/// Stable fingerprint of a schema. Generated bindings embed it and the runtime
/// compares it at start-up, so stale bindings fail loudly instead of
/// mis-decoding. FNV-1a 64 over the canonical (key-sorted) JSON text.
pub fn schema_hash(schema: &Value) -> u64 {
    fnv1a(
        serde_json::to_string(schema)
            .expect("a JSON value always serialises")
            .as_bytes(),
    )
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
