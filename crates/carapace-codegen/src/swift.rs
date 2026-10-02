//! Swift generator: `Codable`, `Sendable` value types namespaced in one enum,
//! conforming to `CarapaceApp` from the CarapaceKit package.

use std::fmt::Write;

use crate::model::{Def, Error, Field, Model, Ty, TypeDef};
use crate::names::{camel, doc_line};

const RESERVED: &[&str] = &[
    "associatedtype",
    "class",
    "deinit",
    "enum",
    "extension",
    "fileprivate",
    "func",
    "import",
    "init",
    "inout",
    "internal",
    "let",
    "open",
    "operator",
    "private",
    "precedencegroup",
    "protocol",
    "public",
    "rethrows",
    "static",
    "struct",
    "subscript",
    "typealias",
    "var",
    "break",
    "case",
    "catch",
    "continue",
    "default",
    "defer",
    "do",
    "else",
    "fallthrough",
    "for",
    "guard",
    "if",
    "in",
    "repeat",
    "return",
    "throw",
    "switch",
    "where",
    "while",
    "Any",
    "as",
    "await",
    "false",
    "is",
    "nil",
    "self",
    "Self",
    "super",
    "throws",
    "true",
    "try",
    "Type",
    "Protocol",
];

fn ident(s: &str) -> String {
    let c = camel(s);
    let c = if c.is_empty() || c.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("_{c}")
    } else {
        c
    };
    if RESERVED.contains(&c.as_str()) {
        format!("`{c}`")
    } else {
        c
    }
}

fn ty(t: &Ty) -> String {
    ty_in(t, "")
}

/// Type spelled with a namespace prefix on every named type, for code outside the namespace enum.
fn ty_in(t: &Ty, prefix: &str) -> String {
    match t {
        Ty::Bool => "Bool".into(),
        Ty::Int(f) if f == "uint64" || f == "uint" => "UInt64".into(),
        Ty::Int(_) => "Int".into(),
        Ty::Float => "Double".into(),
        Ty::Str => "String".into(),
        Ty::Json => "JSONValue".into(),
        Ty::Named(n) => format!("{prefix}{n}"),
        Ty::Optional(i) => format!("{}?", ty_in(i, prefix)),
        Ty::Array(i) => format!("[{}]", ty_in(i, prefix)),
        Ty::Map(i) => format!("[String: {}]", ty_in(i, prefix)),
    }
}

/// How a field is declared in Swift.
enum Kind {
    Required,
    /// `T?`: the field can be absent or null.
    Nil,
    /// `T` with a default, from the schema: written as `= literal` and decoded with a fallback.
    Defaulted(String),
}

fn literal(f: &Field) -> Option<String> {
    use serde_json::Value as V;
    match (&f.ty, f.default.as_ref()?) {
        (Ty::Bool, V::Bool(b)) => Some(b.to_string()),
        (Ty::Int(_), V::Number(n)) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        (Ty::Float, V::Number(n)) => n.as_f64().map(|x| format!("{x:?}")),
        (Ty::Str, V::String(s)) => Some(format!("{s:?}")),
        (Ty::Array(_), V::Array(a)) if a.is_empty() => Some("[]".into()),
        (Ty::Map(_), V::Object(o)) if o.is_empty() => Some("[:]".into()),
        _ => None,
    }
}

/// `out_only`: the type is only ever written by the core, so a default means "always present".
/// `defaults`: this position can carry `= literal` defaults (struct fields, not enum cases).
fn kind(f: &Field, out_only: bool, defaults: bool) -> Kind {
    if matches!(f.ty, Ty::Optional(_)) {
        return Kind::Nil;
    }
    if !f.optional {
        return Kind::Required;
    }
    if f.default.is_some() && out_only {
        return Kind::Required;
    }
    match (defaults, literal(f)) {
        (true, Some(l)) => Kind::Defaulted(l),
        _ => Kind::Nil,
    }
}

/// Field type as declared in Swift, and whether it is Optional.
fn field_ty(f: &Field, out_only: bool, defaults: bool) -> (String, bool) {
    match kind(f, out_only, defaults) {
        Kind::Required | Kind::Defaulted(_) => (ty(&f.ty), false),
        Kind::Nil => match &f.ty {
            Ty::Optional(_) => (ty(&f.ty), true),
            t => (format!("{}?", ty(t)), true),
        },
    }
}

/// The type to name inside `decode(_:forKey:)` for an optional field.
fn inner_ty(f: &Field, prefix: &str) -> String {
    match &f.ty {
        Ty::Optional(i) => ty_in(i, prefix),
        t => ty_in(t, prefix),
    }
}

fn doc(out: &mut String, indent: &str, d: &Option<String>) {
    if let Some(d) = d {
        for line in d.lines() {
            let _ = writeln!(out, "{indent}/// {}", doc_line(line));
        }
    }
}

/// Names the generated code relies on. A Rust type with one of these names would shadow it.
const RESERVED_TYPES: &[&str] = &[
    "Any",
    "Type",
    "Protocol",
    "Self",
    "Bool",
    "Int",
    "UInt64",
    "Double",
    "String",
    "Optional",
    "Array",
    "Dictionary",
    "Key",
    "CodingKeys",
    "Decoder",
    "Encoder",
    "CodingKey",
    "Codable",
    "Hashable",
    "Sendable",
    "Equatable",
    "CaseIterable",
    "JSONValue",
    "NoEvent",
    "NoQuery",
    "NoAnswer",
    "Foundation",
    "CarapaceKit",
    "Swift",
];

fn check_names(m: &Model) -> Result<(), Error> {
    for t in &m.types {
        if RESERVED_TYPES.contains(&t.name.as_str()) && !(t.name == "NoEvent" && m.event.is_none())
        {
            return Err(Error(format!(
                "the Rust type {} has a name the generated Swift needs; rename it (for example {}Info)",
                t.name, t.name
            )));
        }
    }
    // The aliases State, Action, Config, Event, Query, Answer must not collide with another type.
    let roots: [(&str, Option<&Ty>); 6] = [
        ("State", Some(&m.state)),
        ("Action", Some(&m.action)),
        ("Config", Some(&m.config)),
        ("Event", m.event.as_ref()),
        ("Query", m.query.as_ref().map(|q| &q.0)),
        ("Answer", m.query.as_ref().map(|q| &q.1)),
    ];
    for (alias, root) in roots {
        let is_root = matches!(root, Some(Ty::Named(n)) if n == alias);
        if m.index.contains_key(alias) && !is_root {
            return Err(Error(format!(
                "a Rust type named {alias} exists but is not the app's {alias} type; the generated Swift reserves that name, so rename the type"
            )));
        }
    }
    // A Swift struct cannot hold itself without indirection.
    for t in &m.types {
        if let Def::Struct { .. } = &t.def {
            let mut seen = vec![t.name.clone()];
            if let Some(path) = struct_cycle(m, &t.name, &t.name, &mut seen) {
                return Err(Error(format!(
                    "the struct {} contains itself without indirection ({}); a Swift struct cannot. Hold it in a Vec or an enum variant instead",
                    t.name,
                    path.join(" -> ")
                )));
            }
        }
    }
    Ok(())
}

/// Follows direct and Optional struct fields (not Vec, map or enum, which are indirect in Swift).
fn struct_cycle(m: &Model, target: &str, at: &str, seen: &mut Vec<String>) -> Option<Vec<String>> {
    let &i = m.index.get(at)?;
    let Def::Struct { fields } = &m.types[i].def else {
        return None;
    };
    for f in fields {
        let inner = match &f.ty {
            Ty::Named(n) => n,
            Ty::Optional(o) => match &**o {
                Ty::Named(n) => n,
                _ => continue,
            },
            _ => continue,
        };
        if inner == target {
            let mut path = seen.clone();
            path.push(inner.clone());
            return Some(path);
        }
        if !seen.contains(inner) {
            seen.push(inner.clone());
            if let Some(p) = struct_cycle(m, target, inner, seen) {
                return Some(p);
            }
            seen.pop();
        }
    }
    None
}

pub fn generate(m: &Model) -> Result<String, Error> {
    check_names(m)?;
    let app = &m.app;
    let mut o = String::new();
    let mut ext = String::new();
    let _ = writeln!(
        o,
        "// Generated by cargo-carapace from the {app} core schema. Do not edit."
    );
    let _ = writeln!(o, "// Regenerate with: cargo carapace gen swift\n");
    let _ = writeln!(o, "import Foundation\nimport CarapaceKit\n");
    let _ = writeln!(o, "public enum {app}: CarapaceApp {{");
    let _ = writeln!(o, "    public static let name = \"{app}\"");
    let _ = writeln!(
        o,
        "    public static let schemaHash: UInt64 = 0x{:016x}\n",
        m.hash
    );

    for (alias, root) in [
        ("State", &m.state),
        ("Action", &m.action),
        ("Config", &m.config),
    ] {
        match root {
            Ty::Named(n) if n == alias => {}
            Ty::Named(n) => {
                let _ = writeln!(o, "    public typealias {alias} = {n}");
            }
            _ => {
                return Err(Error(format!(
                    "the app's {alias} type must be a struct or enum, not an inline type"
                )))
            }
        }
    }
    match &m.event {
        None => {
            let _ = writeln!(o, "    public typealias Event = NoEvent");
        }
        Some(Ty::Named(n)) if n == "Event" => {}
        Some(Ty::Named(n)) => {
            let _ = writeln!(o, "    public typealias Event = {n}");
        }
        Some(_) => return Err(Error("the app's Event type must be an enum".into())),
    }
    match &m.query {
        None => {
            let _ = writeln!(
                o,
                "    public typealias Query = NoQuery\n    public typealias Answer = NoAnswer"
            );
        }
        Some((q, a)) => {
            for (alias, ty) in [("Query", q), ("Answer", a)] {
                match ty {
                    Ty::Named(n) if n == alias => {}
                    Ty::Named(n) => {
                        let _ = writeln!(o, "    public typealias {alias} = {n}");
                    }
                    _ => {
                        return Err(Error(format!(
                            "the app's {alias} type must be a struct or enum, not an inline type"
                        )))
                    }
                }
            }
        }
    }
    o.push('\n');

    for t in &m.types {
        if m.event.is_none() && t.name == "NoEvent" {
            continue;
        }
        emit_type(&mut o, &mut ext, app, t, m.output_only.contains(&t.name));
    }
    o.push_str("}\n\n");
    o.push_str(&ext);
    let _ = writeln!(o, "public typealias {app}Store = Store<{app}>");
    Ok(o)
}

fn emit_type(o: &mut String, ext: &mut String, app: &str, t: &TypeDef, out_only: bool) {
    doc(o, "    ", &t.doc);
    match &t.def {
        Def::Cases { cases } => {
            let _ = writeln!(
                o,
                "    public enum {}: String, Codable, Sendable, Hashable, CaseIterable {{",
                t.name
            );
            for c in cases {
                let name = ident(c);
                if name.trim_matches('`') == c {
                    let _ = writeln!(o, "        case {name}");
                } else {
                    let _ = writeln!(o, "        case {name} = \"{c}\"");
                }
            }
            o.push_str("    }\n\n");
        }
        Def::Struct { fields } => emit_struct(o, &t.name, fields, out_only),
        Def::Union { tag_key, variants } => {
            let name = &t.name;
            let _ = writeln!(o, "    public indirect enum {name}: Sendable, Hashable {{");
            for v in variants {
                doc(o, "        ", &v.doc);
                if v.fields.is_empty() {
                    let _ = writeln!(o, "        case {}", ident(&v.tag));
                } else {
                    let args: Vec<String> = v
                        .fields
                        .iter()
                        .map(|f| {
                            let (t, opt) = field_ty(f, out_only, false);
                            format!("{}: {t}{}", ident(&f.json), if opt { " = nil" } else { "" })
                        })
                        .collect();
                    let _ = writeln!(o, "        case {}({})", ident(&v.tag), args.join(", "));
                }
            }
            o.push_str("    }\n\n");
            emit_union_codable(ext, app, name, tag_key, variants, out_only);
        }
    }
}

/// `Codable` for an internally tagged union, written as an extension after the namespace.
fn emit_union_codable(
    e: &mut String,
    app: &str,
    name: &str,
    tag_key: &str,
    variants: &[crate::model::Variant],
    out_only: bool,
) {
    let _ = writeln!(e, "extension {app}.{name}: Codable {{");
    e.push_str("    private struct Key: Swift.CodingKey {\n");
    e.push_str("        var stringValue: String\n        var intValue: Int? { nil }\n");
    e.push_str("        init(_ s: String) { stringValue = s }\n");
    e.push_str("        init?(stringValue: String) { self.stringValue = stringValue }\n");
    e.push_str("        init?(intValue: Int) { nil }\n    }\n\n");
    e.push_str("    public init(from decoder: Swift.Decoder) throws {\n");
    e.push_str("        let c = try decoder.container(keyedBy: Key.self)\n");
    let _ = writeln!(
        e,
        "        let tag = try c.decode(String.self, forKey: Key(\"{tag_key}\"))"
    );
    e.push_str("        switch tag {\n");
    for v in variants {
        let case = ident(&v.tag);
        if v.fields.is_empty() {
            let _ = writeln!(e, "        case \"{}\": self = .{case}", v.tag);
        } else {
            let _ = writeln!(e, "        case \"{}\":", v.tag);
            let mut args = Vec::new();
            for (i, f) in v.fields.iter().enumerate() {
                let (_, opt) = field_ty(f, out_only, false);
                let call = if opt { "decodeIfPresent" } else { "decode" };
                let _ = writeln!(
                    e,
                    "            let v{i} = try c.{call}({}.self, forKey: Key(\"{}\"))",
                    inner_ty(f, &format!("{app}.")),
                    f.json
                );
                args.push(format!("{}: v{i}", ident(&f.json)));
            }
            let _ = writeln!(e, "            self = .{case}({})", args.join(", "));
        }
    }
    let _ = writeln!(
        e,
        "        default:\n            throw Swift.DecodingError.dataCorruptedError(forKey: Key(\"{tag_key}\"), in: c, debugDescription: \"unknown {name} {tag_key} \\(tag)\")\n        }}\n    }}\n"
    );
    e.push_str("    public func encode(to encoder: Swift.Encoder) throws {\n");
    e.push_str("        var c = encoder.container(keyedBy: Key.self)\n        switch self {\n");
    for v in variants {
        let case = ident(&v.tag);
        if v.fields.is_empty() {
            let _ = writeln!(e, "        case .{case}:\n            try c.encode(\"{}\", forKey: Key(\"{tag_key}\"))", v.tag);
        } else {
            let binds: Vec<String> = (0..v.fields.len()).map(|i| format!("let v{i}")).collect();
            let _ = writeln!(e, "        case .{case}({}):", binds.join(", "));
            let _ = writeln!(
                e,
                "            try c.encode(\"{}\", forKey: Key(\"{tag_key}\"))",
                v.tag
            );
            for (i, f) in v.fields.iter().enumerate() {
                let call = if field_ty(f, out_only, false).1 {
                    "encodeIfPresent"
                } else {
                    "encode"
                };
                let _ = writeln!(
                    e,
                    "            try c.{call}(v{i}, forKey: Key(\"{}\"))",
                    f.json
                );
            }
        }
    }
    e.push_str("        }\n    }\n}\n\n");
}

fn emit_struct(o: &mut String, name: &str, fields: &[Field], out_only: bool) {
    let kinds: Vec<Kind> = fields.iter().map(|f| kind(f, out_only, true)).collect();
    let _ = writeln!(
        o,
        "    public struct {name}: Codable, Sendable, Hashable {{"
    );
    for f in fields {
        doc(o, "        ", &f.doc);
        let _ = writeln!(
            o,
            "        public var {}: {}",
            ident(&f.json),
            field_ty(f, out_only, true).0
        );
    }
    let params: Vec<String> = fields
        .iter()
        .zip(&kinds)
        .map(|(f, k)| {
            let (t, _) = field_ty(f, out_only, true);
            match k {
                Kind::Required => format!("{}: {t}", ident(&f.json)),
                Kind::Nil => format!("{}: {t} = nil", ident(&f.json)),
                Kind::Defaulted(l) => format!("{}: {t} = {l}", ident(&f.json)),
            }
        })
        .collect();
    let _ = writeln!(o, "\n        public init({}) {{", params.join(", "));
    for f in fields {
        let n = ident(&f.json);
        let _ = writeln!(o, "            self.{n} = {n}");
    }
    o.push_str("        }\n");
    let needs_decoder = kinds.iter().any(|k| matches!(k, Kind::Defaulted(_)));
    if needs_decoder
        || fields
            .iter()
            .any(|f| ident(&f.json).trim_matches('`') != f.json)
    {
        o.push_str("\n        enum CodingKeys: Swift.String, Swift.CodingKey {\n");
        for f in fields {
            let n = ident(&f.json);
            if n.trim_matches('`') == f.json {
                let _ = writeln!(o, "            case {n}");
            } else {
                let _ = writeln!(o, "            case {n} = \"{}\"", f.json);
            }
        }
        o.push_str("        }\n");
    }
    if needs_decoder {
        o.push_str("\n        public init(from decoder: Swift.Decoder) throws {\n");
        o.push_str("            let c = try decoder.container(keyedBy: CodingKeys.self)\n");
        for (f, k) in fields.iter().zip(&kinds) {
            let n = ident(&f.json);
            let t = inner_ty(f, "");
            let _ = match k {
                Kind::Required => writeln!(
                    o,
                    "            self.{n} = try c.decode({t}.self, forKey: .{n})"
                ),
                Kind::Nil => writeln!(
                    o,
                    "            self.{n} = try c.decodeIfPresent({t}.self, forKey: .{n})"
                ),
                Kind::Defaulted(l) => writeln!(
                    o,
                    "            self.{n} = try c.decodeIfPresent({t}.self, forKey: .{n}) ?? {l}"
                ),
            };
        }
        o.push_str("        }\n");
    }
    o.push_str("    }\n\n");
}
