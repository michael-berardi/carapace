//! Typed bindings from a Carapace schema bundle.
//!
//! ```
//! # let schema = r##"{"carapace":1,"name":"Demo","config":{"$ref":"#/definitions/C"},"state":{"$ref":"#/definitions/S"},
//! # "action":{"$ref":"#/definitions/A"},"event":{"$ref":"#/definitions/E"},"definitions":{
//! # "C":{"type":"object"},"S":{"type":"object","properties":{"n":{"type":"integer","format":"int64"}},"required":["n"]},
//! # "A":{"oneOf":[{"type":"object","properties":{"type":{"const":"go","type":"string"}},"required":["type"]}]},
//! # "E":{"oneOf":[]}}}"##;
//! let files = carapace_codegen::generate(schema).unwrap();
//! assert!(files.swift.contains("public enum Demo: CarapaceApp"));
//! assert!(files.typescript.contains("export interface S"));
//! ```

mod model;
mod names;
mod parse;
mod swift;
mod typescript;

pub use model::{Error, Model};

/// Generated source for each language.
pub struct Generated {
    pub swift: String,
    pub typescript: String,
    pub app: String,
    pub hash: u64,
}

/// FNV-1a 64 over the canonical JSON text; identical to `carapace::schema_hash`.
pub fn schema_hash(schema: &serde_json::Value) -> u64 {
    let text = serde_json::to_string(schema).expect("a JSON value always serialises");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn parse(schema: &str) -> Result<Model, Error> {
    parse::parse(schema, schema_hash)
}

pub fn generate(schema: &str) -> Result<Generated, Error> {
    let model = parse(schema)?;
    Ok(Generated {
        swift: swift::generate(&model)?,
        typescript: typescript::generate(&model)?,
        app: model.app.clone(),
        hash: model.hash,
    })
}

#[cfg(test)]
mod tests;
