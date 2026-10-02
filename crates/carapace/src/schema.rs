//! The machine-readable contract between a core and its shells.

use schemars::generate::SchemaSettings;
use serde_json::{json, Value};

use crate::engine::{App, Queries};

/// Bumped when the C ABI or the wire format changes incompatibly.
pub const ABI_VERSION: u32 = 1;

/// JSON Schema bundle describing an app's config, state, actions and events.
pub fn schema<A: App>() -> Value {
    bundle::<A>(|_, _| None)
}

/// Like [`schema`], plus the app's pure query and answer types.
pub fn schema_with_queries<A: Queries>() -> Value {
    bundle::<A>(|input, output| {
        Some((
            input.subschema_for::<A::Query>(),
            output.subschema_for::<A::Answer>(),
        ))
    })
}

type QuerySchemas = Option<(schemars::Schema, schemars::Schema)>;

/// Types the shell sends (config, action, query) are described as serde reads them; types the
/// core sends (state, event, answer) as serde writes them, so `skip_serializing_if` fields are
/// optional for the shell. A type used in both directions gets its input shape, which is the
/// more permissive one for decoding.
fn bundle<A: App>(
    queries: impl FnOnce(&mut schemars::SchemaGenerator, &mut schemars::SchemaGenerator) -> QuerySchemas,
) -> Value {
    let mut input = SchemaSettings::draft07().into_generator();
    let mut output = SchemaSettings::draft07().for_serialize().into_generator();
    let config = input.subschema_for::<A::Config>();
    let action = input.subschema_for::<A::Action>();
    let state = output.subschema_for::<A::State>();
    let event = output.subschema_for::<A::Event>();
    let queries = queries(&mut input, &mut output);
    let mut definitions = output.take_definitions(true);
    for (name, shape) in input.take_definitions(true) {
        if let Some(other) = definitions.get(&name) {
            // The same Rust type can differ between directions (`required`, `default`, fields hidden
            // with `skip_serializing`); two different types that share a name cannot be reconciled.
            assert!(
                same_type(other, &shape),
                "carapace: {}: two different types are both named {name:?}, one sent by the shell and one by the core. Rename one of them.",
                A::NAME
            );
        }
        definitions.insert(name, shape);
    }
    let mut out = json!({
        "carapace": ABI_VERSION,
        "name": A::NAME,
        "config": config,
        "state": state,
        "action": action,
        "event": event,
        "definitions": Value::Object(definitions),
    });
    if let Some((query, answer)) = queries {
        out["query"] = json!(query);
        out["answer"] = json!(answer);
    }
    out
}

/// Stable fingerprint of a schema. Generated bindings embed it and the runtime
/// compares it at start-up, so stale bindings fail loudly instead of
/// mis-decoding. FNV-1a 64 over the JSON text, keys in declaration order (the codegen reproduces it exactly).
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

/// The schema without the keywords that legitimately differ between serialising and deserialising.
fn strip_directional(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "required" | "default"))
                .map(|(k, v)| (k.clone(), strip_directional(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_directional).collect()),
        other => other.clone(),
    }
}

/// Whether two definitions plausibly describe one Rust type seen from the two serde directions:
/// same kind, and for objects one field set contains the other.
fn same_type(a: &Value, b: &Value) -> bool {
    let kind = |v: &Value| {
        (
            v.get("oneOf").is_some(),
            v.get("enum").is_some(),
            v.get("properties").is_some(),
        )
    };
    if kind(a) != kind(b) {
        return false;
    }
    if let (Some(va), Some(vb)) = (
        a.get("oneOf").and_then(Value::as_array),
        b.get("oneOf").and_then(Value::as_array),
    ) {
        // Enums: same variants in the same order, each variant's fields related as for structs.
        return va.len() == vb.len() && va.iter().zip(vb).all(|(x, y)| same_type(x, y));
    }
    match (
        a.get("properties").and_then(Value::as_object),
        b.get("properties").and_then(Value::as_object),
    ) {
        (Some(pa), Some(pb)) => {
            pa.keys().all(|k| pb.contains_key(k)) || pb.keys().all(|k| pa.contains_key(k))
        }
        _ => strip_directional(a) == strip_directional(b),
    }
}
