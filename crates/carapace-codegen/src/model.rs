//! Language-neutral model of an app's types, parsed from its JSON Schema.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Bool,
    /// Integer with the schema's format (`int64`, `uint32`, ...).
    Int(String),
    Float,
    Str,
    /// Arbitrary JSON.
    Json,
    Named(String),
    Optional(Box<Ty>),
    Array(Box<Ty>),
    Map(Box<Ty>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Name exactly as it appears in JSON.
    pub json: String,
    pub ty: Ty,
    /// May be absent from the JSON (not in `required`, or has a default).
    pub optional: bool,
    /// The schema's `default`, when it has one.
    pub default: Option<serde_json::Value>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// Tag value exactly as it appears in JSON.
    pub tag: String,
    pub fields: Vec<Field>,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Def {
    Struct {
        fields: Vec<Field>,
    },
    /// An internally tagged union (`#[serde(tag = "type")]`).
    Union {
        tag_key: String,
        variants: Vec<Variant>,
    },
    /// A fieldless enum serialised as strings.
    Cases {
        cases: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub doc: Option<String>,
    pub def: Def,
}

#[derive(Debug)]
pub struct Model {
    /// App name from `App::NAME`.
    pub app: String,
    pub hash: u64,
    pub types: Vec<TypeDef>,
    pub config: Ty,
    pub state: Ty,
    pub action: Ty,
    /// `None` when the app uses `NoEvent`.
    pub event: Option<Ty>,
    /// Pure query and answer types, when the core exports queries.
    pub query: Option<(Ty, Ty)>,
    /// Types in `types` by name, for lookups.
    pub index: BTreeMap<String, usize>,
    /// Types only ever sent by the core (reachable from state, event and answer, never from
    /// config, action or query). The core always writes their fields, so defaults make them
    /// required in the shells.
    pub output_only: std::collections::BTreeSet<String>,
}

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}
