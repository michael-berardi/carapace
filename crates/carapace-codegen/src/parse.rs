//! JSON Schema (as written by `carapace::schema`) -> [`Model`].
//!
//! Only the constructs serde + schemars produce for the documented conventions
//! are accepted. Anything else fails with the path and the fix.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::model::{err, Def, Error, Field, Model, Ty, TypeDef, Variant};

/// Parse a schema bundle. `hash_fn` computes the bundle fingerprint.
pub fn parse(text: &str, hash_fn: fn(&Value) -> u64) -> Result<Model, Error> {
    let root: Value =
        serde_json::from_str(text).map_err(|e| Error(format!("schema is not valid JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| Error("schema root must be an object".into()))?;
    match obj.get("carapace").and_then(Value::as_u64) {
        Some(1) => {}
        Some(v) => {
            return err(format!(
                "schema ABI version {v} is not supported (this tool reads 1)"
            ))
        }
        None => return err("not a Carapace schema bundle (missing \"carapace\" version)"),
    }
    let app = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error("schema has no app name".into()))?;
    let defs = obj
        .get("definitions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let mut p = Parser {
        defs: &defs,
        out: BTreeMap::new(),
        order: Vec::new(),
        stack: Vec::new(),
    };
    let state = p.ty(obj.get("state").unwrap_or(&Value::Null), "state")?;
    let action = p.ty(obj.get("action").unwrap_or(&Value::Null), "action")?;
    let config = p.ty(obj.get("config").unwrap_or(&Value::Null), "config")?;
    let event_schema = obj.get("event").unwrap_or(&Value::Null);
    let event = if is_no_event(event_schema, &defs) {
        None
    } else {
        Some(p.ty(event_schema, "event")?)
    };

    let query = match (obj.get("query"), obj.get("answer")) {
        (Some(q), Some(a)) => Some((p.ty(q, "query")?, p.ty(a, "answer")?)),
        _ => None,
    };

    let mut types = Vec::new();
    let mut index = BTreeMap::new();
    for name in &p.order {
        index.insert(name.clone(), types.len());
        types.push(p.out[name].clone());
    }
    let mut model = Model {
        app: app.to_string(),
        hash: hash_fn(&root),
        types,
        config,
        state,
        action,
        event,
        query,
        index,
        output_only: Default::default(),
    };
    model.output_only = output_only(&model);
    Ok(model)
}

/// `NoEvent` is an empty tagged enum: a `oneOf` with no variants.
fn is_no_event(schema: &Value, defs: &Map<String, Value>) -> bool {
    let target = match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => r.rsplit('/').next().and_then(|n| defs.get(n)),
        None => Some(schema),
    };
    target
        .and_then(|t| t.get("oneOf").or_else(|| t.get("enum")))
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

struct Parser<'a> {
    defs: &'a Map<String, Value>,
    out: BTreeMap<String, TypeDef>,
    order: Vec<String>,
    stack: Vec<String>,
}

impl Parser<'_> {
    fn ty(&mut self, s: &Value, path: &str) -> Result<Ty, Error> {
        if let Some(r) = s.get("$ref").and_then(Value::as_str) {
            let name = r.rsplit('/').next().unwrap_or(r).to_string();
            self.named(&name, path)?;
            return Ok(Ty::Named(name));
        }
        // allOf with a single $ref carries a description in schemars output.
        if let Some(all) = s.get("allOf").and_then(Value::as_array) {
            if all.len() == 1 {
                return self.ty(&all[0], path);
            }
        }
        if let Some(any) = s.get("anyOf").and_then(Value::as_array) {
            let non_null: Vec<&Value> = any
                .iter()
                .filter(|v| v.get("type").and_then(Value::as_str) != Some("null"))
                .collect();
            if non_null.len() == 1 && non_null.len() < any.len() {
                return Ok(Ty::Optional(Box::new(self.ty(non_null[0], path)?)));
            }
            return err(format!("{path}: untagged unions (anyOf) are not supported; use #[serde(tag = \"type\")] on the enum"));
        }
        match s.get("type") {
            Some(Value::Array(types)) => {
                let names: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
                let non_null: Vec<&&str> = names.iter().filter(|t| **t != "null").collect();
                if non_null.len() == 1 && names.len() == 2 {
                    let mut inner = s.clone();
                    inner["type"] = Value::String((*non_null[0]).to_string());
                    return Ok(Ty::Optional(Box::new(self.ty(&inner, path)?)));
                }
                err(format!("{path}: type {names:?} is not supported"))
            }
            Some(Value::String(t)) => self.by_type(t, s, path),
            _ => {
                if s.as_object().is_some_and(|o| o.is_empty()) || s == &Value::Bool(true) {
                    return Ok(Ty::Json);
                }
                err(format!(
                    "{path}: cannot determine type from schema {}",
                    short(s)
                ))
            }
        }
    }

    fn by_type(&mut self, t: &str, s: &Value, path: &str) -> Result<Ty, Error> {
        Ok(match t {
            "boolean" => Ty::Bool,
            "string" => Ty::Str,
            "number" => Ty::Float,
            "integer" => Ty::Int(s.get("format").and_then(Value::as_str).unwrap_or("int64").to_string()),
            "array" => match s.get("items") {
                Some(items) if items.is_object() => Ty::Array(Box::new(self.ty(items, &format!("{path}[]"))?)),
                _ => return err(format!("{path}: tuples are not supported; use a struct or Vec<T>")),
            },
            "object" => match s.get("additionalProperties") {
                Some(v) if v.is_object() => Ty::Map(Box::new(self.ty(v, &format!("{path}{{}}"))?)),
                _ if s.get("properties").is_none() => Ty::Json,
                _ => return err(format!("{path}: anonymous inline struct; give the struct a name so it gets a definition")),
            },
            "null" => return err(format!("{path}: a bare null type is not supported")),
            other => return err(format!("{path}: unknown schema type {other:?}")),
        })
    }

    fn named(&mut self, name: &str, path: &str) -> Result<(), Error> {
        if self.out.contains_key(name) || self.stack.iter().any(|n| n == name) {
            return Ok(());
        }
        let schema = self
            .defs
            .get(name)
            .ok_or_else(|| Error(format!("{path}: reference to missing definition {name:?}")))?
            .clone();
        self.stack.push(name.to_string());
        let doc = schema
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let def = self.def(name, &schema)?;
        self.stack.pop();
        self.order.push(name.to_string());
        self.out.insert(
            name.to_string(),
            TypeDef {
                name: name.to_string(),
                doc,
                def,
            },
        );
        Ok(())
    }

    fn def(&mut self, name: &str, s: &Value) -> Result<Def, Error> {
        if let Some(cases) = s.get("enum").and_then(Value::as_array) {
            let cases: Option<Vec<String>> = cases
                .iter()
                .map(|c| c.as_str().map(str::to_string))
                .collect();
            return cases
                .map(|cases| Def::Cases { cases })
                .ok_or_else(|| Error(format!("{name}: enums must have string values")));
        }
        if let Some(one) = s.get("oneOf").and_then(Value::as_array) {
            return self.one_of(name, one);
        }
        if s.get("anyOf").is_some() {
            return err(format!("{name}: untagged unions (anyOf) are not supported; use #[serde(tag = \"type\")] on the enum"));
        }
        if s.get("type").and_then(Value::as_str) == Some("object") {
            return Ok(Def::Struct {
                fields: self.fields(name, s, None)?,
            });
        }
        err(format!(
            "{name}: unsupported definition {}. Structs, string enums and #[serde(tag = \"type\")] enums are supported",
            short(s)
        ))
    }

    fn one_of(&mut self, name: &str, one: &[Value]) -> Result<Def, Error> {
        // Fieldless enums with doc comments become oneOf of string consts.
        if !one.is_empty()
            && one
                .iter()
                .all(|v| v.get("const").is_some() && v.get("properties").is_none())
        {
            let cases: Option<Vec<String>> = one
                .iter()
                .map(|v| v["const"].as_str().map(str::to_string))
                .collect();
            if let Some(cases) = cases {
                return Ok(Def::Cases { cases });
            }
        }
        let mut tag_key: Option<String> = None;
        let mut variants = Vec::new();
        for (i, v) in one.iter().enumerate() {
            let props = v.get("properties").and_then(Value::as_object);
            let found = props.and_then(|p| {
                p.iter().find_map(|(k, pv)| {
                    pv.get("const")
                        .and_then(Value::as_str)
                        .map(|c| (k.clone(), c.to_string()))
                })
            });
            let Some((key, tag)) = found else {
                return err(format!(
                    "{name}: variant {i} is not internally tagged. Put #[serde(tag = \"type\", rename_all = \"camelCase\")] on the enum \
                     (externally tagged, adjacently tagged and untagged enums are not supported)"
                ));
            };
            if tag_key.get_or_insert_with(|| key.clone()) != &key {
                return err(format!("{name}: variants use different tag keys"));
            }
            const KNOWN: [&str; 7] = [
                "type",
                "properties",
                "required",
                "description",
                "allOf",
                "title",
                "additionalProperties",
            ];
            if let Some(extra) = v
                .as_object()
                .and_then(|o| o.keys().find(|k| !KNOWN.contains(&k.as_str())))
            {
                return err(format!(
                    "{name}.{tag}: unsupported schema keyword {extra:?} in an enum variant"
                ));
            }
            let mut fields = self.fields(&format!("{name}.{tag}"), v, Some(&key))?;
            // A newtype variant `Save(Settings)` keeps the struct's fields next to the tag.
            for part in v
                .get("allOf")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let target = part
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|r| r.rsplit('/').next());
                let Some(target) = target else {
                    return err(format!(
                        "{name}.{tag}: unsupported allOf in an enum variant"
                    ));
                };
                self.named(target, &format!("{name}.{tag}"))?;
                match self.out.get(target).map(|t| &t.def) {
                    Some(Def::Struct { fields: inner }) => fields.extend(inner.iter().cloned()),
                    _ => {
                        return err(format!(
                            "{name}.{tag}: the variant wraps {target}, which is not a plain struct. Tagged enums can only wrap structs; give the variant named fields instead"
                        ))
                    }
                }
            }
            let doc = v
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            variants.push(Variant { tag, fields, doc });
        }
        Ok(Def::Union {
            tag_key: tag_key.unwrap_or_else(|| "type".into()),
            variants,
        })
    }

    fn fields(&mut self, name: &str, s: &Value, skip: Option<&str>) -> Result<Vec<Field>, Error> {
        let empty = Map::new();
        let props = s
            .get("properties")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let required: Vec<&str> = s
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // Declaration order is preserved by serde_json's `preserve_order`.
        let names: Vec<&String> = props.keys().filter(|k| Some(k.as_str()) != skip).collect();
        let mut out = Vec::new();
        for key in names {
            let fs = &props[key];
            let ty = self.ty(fs, &format!("{name}.{key}"))?;
            let optional = !required.contains(&key.as_str()) || fs.get("default").is_some();
            let doc = fs
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            let default = fs.get("default").cloned();
            out.push(Field {
                json: key.clone(),
                ty,
                optional,
                default,
                doc,
            });
        }
        Ok(out)
    }
}

fn short(v: &Value) -> String {
    let t = v.to_string();
    if t.len() > 120 {
        format!("{}...", &t[..120])
    } else {
        t
    }
}

fn named_in(t: &Ty, out: &mut Vec<String>) {
    match t {
        Ty::Named(n) => out.push(n.clone()),
        Ty::Optional(i) | Ty::Array(i) | Ty::Map(i) => named_in(i, out),
        _ => {}
    }
}

fn reach(model: &Model, roots: &[&Ty]) -> std::collections::BTreeSet<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = Vec::new();
    for r in roots {
        named_in(r, &mut stack);
    }
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        let Some(&i) = model.index.get(&n) else {
            continue;
        };
        match &model.types[i].def {
            Def::Struct { fields } => fields.iter().for_each(|f| named_in(&f.ty, &mut stack)),
            Def::Union { variants, .. } => variants
                .iter()
                .flat_map(|v| &v.fields)
                .for_each(|f| named_in(&f.ty, &mut stack)),
            Def::Cases { .. } => {}
        }
    }
    seen
}

fn output_only(m: &Model) -> std::collections::BTreeSet<String> {
    let mut input = vec![&m.config, &m.action];
    if let Some((q, _)) = &m.query {
        input.push(q);
    }
    let mut output = vec![&m.state];
    if let Some(e) = &m.event {
        output.push(e);
    }
    if let Some((_, a)) = &m.query {
        output.push(a);
    }
    let sent = reach(m, &input);
    reach(m, &output)
        .into_iter()
        .filter(|n| !sent.contains(n))
        .collect()
}
