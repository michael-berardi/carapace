use super::*;

fn schema(defs: &str, action: &str) -> String {
    format!(
        r##"{{"carapace":1,"name":"Demo","config":{{"$ref":"#/definitions/Config"}},"state":{{"$ref":"#/definitions/State"}},
        "action":{{"$ref":"#/definitions/{action}"}},"event":{{"$ref":"#/definitions/NoEvent"}},
        "definitions":{{"Config":{{"type":"object"}},"NoEvent":{{"oneOf":[]}},{defs}}}}}"##
    )
}

const STATE: &str = r##""State":{"type":"object","properties":{"a_b":{"type":"string"},"repeat":{"type":"integer","format":"uint64"},
  "tags":{"type":"array","items":{"type":"string"}},"opt":{"type":["number","null"]},
  "kv":{"type":"object","additionalProperties":{"type":"boolean"}},"any":{}},"required":["a_b","repeat","tags","kv"]}"##;

#[test]
fn rejects_externally_tagged_enums_with_the_fix() {
    let s = schema(
        &format!(
            r##"{STATE},"Action":{{"oneOf":[{{"type":"object","properties":{{"Go":{{"type":"integer"}}}},"required":["Go"]}}]}}"##
        ),
        "Action",
    );
    let e = generate(&s).err().unwrap().to_string();
    assert!(e.contains("serde(tag = \"type\""), "{e}");
}

#[test]
fn rejects_untagged_unions() {
    let s = schema(
        &format!(r##"{STATE},"Action":{{"anyOf":[{{"type":"string"}},{{"type":"integer"}}]}}"##),
        "Action",
    );
    assert!(generate(&s).err().unwrap().to_string().contains("untagged"));
}

#[test]
fn rejects_non_carapace_documents() {
    assert!(generate("{}")
        .err()
        .unwrap()
        .to_string()
        .contains("not a Carapace"));
    assert!(generate("nope")
        .err()
        .unwrap()
        .to_string()
        .contains("not valid JSON"));
}

fn action_union() -> String {
    schema(
        &format!(
            r##"{STATE},"Action":{{"oneOf":[
            {{"type":"object","properties":{{"type":{{"const":"default","type":"string"}}}},"required":["type"]}},
            {{"type":"object","properties":{{"type":{{"const":"set_it","type":"string"}},"in":{{"type":"integer","format":"int32"}},
              "note":{{"type":["string","null"]}}}},"required":["type","in"]}}]}}"##
        ),
        "Action",
    )
}

#[test]
fn swift_escapes_keywords_and_maps_names() {
    let g = generate(&action_union()).unwrap();
    assert!(g.swift.contains("case `default`"), "{}", g.swift);
    assert!(
        g.swift.contains("case setIt(in:") || g.swift.contains("case setIt(`in`:"),
        "{}",
        g.swift
    );
    assert!(g.swift.contains("public var aB: String"));
    assert!(g.swift.contains("case aB = \"a_b\""));
    assert!(g.swift.contains("public var `repeat`: UInt64"));
    assert!(g.swift.contains("public var kv: [String: Bool]"));
    assert!(g.swift.contains("public var any: JSONValue?"));
    assert!(g.swift.contains("typealias Event = NoEvent"));
    assert!(!g.swift.contains("enum NoEvent"));
}

#[test]
fn typescript_models_optionals_unions_and_creators() {
    let g = generate(&action_union()).unwrap();
    assert!(g.typescript.contains("a_b: string;"));
    assert!(g.typescript.contains("opt?: number | null;"));
    assert!(g.typescript.contains("kv: Record<string, boolean>;"));
    assert!(g
        .typescript
        .contains("| { type: \"set_it\"; in: number; note?: string | null }"));
    assert!(g.typescript.contains("event: never;"));
    assert!(
        g.typescript.contains("\"default\": (): Action")
            || g.typescript.contains("default: (): Action")
    );
}

#[test]
fn hash_changes_when_the_schema_changes() {
    let a = generate(&action_union()).unwrap().hash;
    let b = generate(&action_union().replace("set_it", "set_that"))
        .unwrap()
        .hash;
    assert_ne!(a, b);
}

#[test]
fn queries_become_types_and_aliases() {
    let s = action_union().replace(
        r##""event":{"$ref":"#/definitions/NoEvent"},"##,
        r##""event":{"$ref":"#/definitions/NoEvent"},"query":{"$ref":"#/definitions/Q"},"answer":{"$ref":"#/definitions/R"},"##,
    );
    let s = s.replace(
        r##""Config":{"type":"object"},"##,
        r##""Config":{"type":"object"},"Q":{"oneOf":[{"type":"object","properties":{"type":{"const":"gains","type":"string"},"level":{"type":"number"}},"required":["type","level"]}]},"R":{"type":"object","properties":{"r":{"type":"number"}},"required":["r"]},"##,
    );
    let g = generate(&s).unwrap();
    assert!(
        g.swift.contains("public typealias Query = Q"),
        "{}",
        g.swift
    );
    assert!(g.swift.contains("public typealias Answer = R"));
    assert!(g.typescript.contains("query: Q;") && g.typescript.contains("answer: R;"));
    let none = generate(&action_union()).unwrap();
    assert!(none.swift.contains("typealias Query = NoQuery"));
    assert!(none.typescript.contains("query: never;"));
}

#[test]
fn defaults_become_swift_defaults_for_inputs_and_required_for_outputs() {
    // `Config` is sent by the shell; `S` is only written by the core. Both have a defaulted field.
    let s = r##"{"carapace":1,"name":"D","config":{"$ref":"#/definitions/Config"},"state":{"$ref":"#/definitions/S"},
      "action":{"$ref":"#/definitions/A"},"event":{"$ref":"#/definitions/NoEvent"},"definitions":{
      "Config":{"type":"object","properties":{"start":{"type":"integer","format":"int64","default":5},"name":{"type":"string","default":"x"}}},
      "S":{"type":"object","properties":{"n":{"type":"integer","format":"int64","default":0}}},
      "NoEvent":{"oneOf":[]},
      "A":{"oneOf":[{"type":"object","properties":{"type":{"const":"go","type":"string"}},"required":["type"]}]}}}"##;
    let g = generate(s).unwrap();
    assert!(g.swift.contains("public var start: Int\n"), "{}", g.swift);
    assert!(g.swift.contains("start: Int = 5, name: String = \"x\""));
    assert!(g
        .swift
        .contains("decodeIfPresent(Int.self, forKey: .start) ?? 5"));
    // Output-only: required, no Optional and no fallback decoder.
    assert!(g.swift.contains("public var n: Int\n"));
    assert!(!g.swift.contains("forKey: .n) ?? 0"));
    assert!(g.typescript.contains("start?: number;"));
    assert!(g.typescript.contains("  n: number;"));
}

#[test]
fn union_codable_qualifies_named_types_outside_the_namespace() {
    // Regression: `Settings` inside `extension App.Event: Codable` did not resolve, and would
    // have collided with SwiftUI's `Settings` scene anyway.
    let s = schema(
        &format!(
            r##"{STATE},"Settings":{{"type":"object","properties":{{"a":{{"type":"string"}}}},"required":["a"]}},
            "Action":{{"oneOf":[{{"type":"object","properties":{{"type":{{"const":"save","type":"string"}},
            "settings":{{"$ref":"#/definitions/Settings"}},"all":{{"type":"array","items":{{"$ref":"#/definitions/Settings"}}}}}},
            "required":["type","settings","all"]}}]}}"##
        ),
        "Action",
    );
    let g = generate(&s).unwrap();
    assert!(
        g.swift.contains("c.decode(Demo.Settings.self"),
        "{}",
        g.swift
    );
    assert!(g.swift.contains("c.decode([Demo.Settings].self"));
}

fn newtype_action() -> String {
    schema(
        &format!(
            r##"{STATE},"Settings":{{"type":"object","properties":{{"a":{{"type":"string"}},"my-key":{{"type":"integer","format":"int32"}}}},"required":["a","my-key"]}},
            "Action":{{"oneOf":[{{"required":["type"],"type":"object","properties":{{"type":{{"type":"string","const":"save"}}}},
            "allOf":[{{"$ref":"#/definitions/Settings"}}]}}]}}"##
        ),
        "Action",
    )
}

#[test]
fn newtype_variants_keep_the_wrapped_structs_fields() {
    // Regression: `Save(Settings)` used to generate a bare `case save`, dropping the payload.
    let g = generate(&newtype_action()).unwrap();
    assert!(
        g.swift.contains("case save(a: String, myKey: Int)"),
        "{}",
        g.swift
    );
    assert!(
        g.typescript
            .contains(r#"| { type: "save"; a: string; "my-key": number }"#),
        "{}",
        g.typescript
    );
    // Regression: non-identifier field names in action creators must use bracket access.
    assert!(
        g.typescript.contains(r#""my-key": a["my-key"]"#),
        "{}",
        g.typescript
    );
}

#[test]
fn enum_variants_with_unknown_schema_keywords_fail_loudly() {
    let s = newtype_action().replace(
        r##""allOf":[{"$ref":"#/definitions/Settings"}]"##,
        r#""not":{"type":"null"}"#,
    );
    let e = generate(&s).err().unwrap().to_string();
    assert!(e.contains("unsupported schema keyword"), "{e}");
}

#[test]
fn rust_types_named_like_generated_swift_names_are_rejected_with_a_fix() {
    let s = schema(
        &format!(
            r##"{STATE},"Decoder":{{"type":"object","properties":{{"a":{{"type":"string"}}}}}},
            "Action":{{"oneOf":[{{"type":"object","properties":{{"type":{{"const":"go","type":"string"}},"d":{{"$ref":"#/definitions/Decoder"}}}},"required":["type","d"]}}]}}"##
        ),
        "Action",
    );
    let e = generate(&s).err().unwrap().to_string();
    assert!(e.contains("Decoder") && e.contains("rename"), "{e}");
    // A user type named Event while the app has NoEvent would collide with the generated alias.
    let s = schema(
        &format!(
            r##"{STATE},"Event":{{"type":"object","properties":{{"a":{{"type":"string"}}}}}},
            "Action":{{"oneOf":[{{"type":"object","properties":{{"type":{{"const":"go","type":"string"}},"e":{{"$ref":"#/definitions/Event"}}}},"required":["type","e"]}}]}}"##
        ),
        "Action",
    );
    let e = generate(&s).err().unwrap().to_string();
    assert!(e.contains("Event") && e.contains("rename"), "{e}");
}

#[test]
fn a_struct_that_holds_itself_without_indirection_is_rejected_and_enums_are_indirect() {
    let s = schema(
        &format!(
            r##"{STATE},"Node":{{"type":"object","properties":{{"next":{{"anyOf":[{{"$ref":"#/definitions/Node"}},{{"type":"null"}}]}}}}}},
            "Action":{{"oneOf":[{{"type":"object","properties":{{"type":{{"const":"go","type":"string"}},"n":{{"$ref":"#/definitions/Node"}}}},"required":["type","n"]}}]}}"##
        ),
        "Action",
    );
    let e = generate(&s).err().unwrap().to_string();
    assert!(e.contains("Node") && e.contains("itself"), "{e}");
    let ok = generate(&action_union()).unwrap();
    assert!(ok.swift.contains("public indirect enum Action"));
}
