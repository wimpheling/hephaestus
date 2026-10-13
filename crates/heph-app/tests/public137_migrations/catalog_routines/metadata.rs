//! Only named, typed OID fields are mapped to their captured qualified identities.
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const RAW_FIELDS: [&str; 30] = [
    "oid",
    "proname",
    "pronamespace",
    "proowner",
    "prolang",
    "procost",
    "prorows",
    "provariadic",
    "prosupport",
    "prokind",
    "prosecdef",
    "proleakproof",
    "proisstrict",
    "proretset",
    "provolatile",
    "proparallel",
    "pronargs",
    "pronargdefaults",
    "prorettype",
    "proargtypes",
    "proallargtypes",
    "proargmodes",
    "proargnames",
    "proargdefaults",
    "protrftypes",
    "prosrc",
    "probin",
    "prosqlbody",
    "proconfig",
    "proacl",
];

pub fn keys(value: &Value, expected: &[&str]) {
    let actual: BTreeSet<_> = object(value).keys().map(String::as_str).collect();
    assert_eq!(actual, expected.iter().copied().collect());
}

pub fn object(value: &Value) -> &Map<String, Value> {
    value
        .as_object()
        .expect("expected a complete captured object")
}

pub fn text(value: &Value) -> &str {
    value.as_str().expect("expected PostgreSQL text")
}

pub fn oid(value: &Value) -> &str {
    let text = text(value);
    assert!(!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()));
    assert!(!text.starts_with('0'));
    assert!(text.parse::<u32>().expect("OID must fit u32") > 0);
    text
}

pub fn type_name(identity: &Value) -> String {
    keys(identity, &["type", "schema", "name", "identity"]);
    assert_eq!(identity["type"], "type");
    format!("{}.{}", text(&identity["schema"]), text(&identity["name"]))
}

pub fn signature(name: &str, types: &[String]) -> String {
    format!("public.{name}({})", types.join(","))
}

fn arguments(routine: &Value) -> Vec<&Value> {
    let raw = &routine["raw"];
    let arguments = routine["argument_types"].as_array();
    let arguments: Vec<_> = arguments.into_iter().flatten().collect();
    let all = if raw["proallargtypes"].is_null() {
        &raw["proargtypes"]
    } else {
        &raw["proallargtypes"]
    };
    let oids = all.as_array().expect("PG17 oid/oidvector JSON array");
    assert_eq!(arguments.len(), oids.len());
    for (index, (argument, raw_oid)) in arguments.iter().zip(oids).enumerate() {
        keys(argument, &["position", "oid", "identity"]);
        assert_eq!(
            argument["position"].as_u64(),
            Some(u64::try_from(index + 1).unwrap())
        );
        assert_eq!(oid(&argument["oid"]), oid(raw_oid));
        type_name(&argument["identity"]);
    }
    arguments
}

pub fn input_types(routine: &Value) -> Vec<String> {
    let raw = &routine["raw"];
    let arguments = arguments(routine);
    let modes = raw["proargmodes"].as_array();
    if let Some(modes) = modes {
        assert_eq!(modes.len(), arguments.len());
    }
    let mut input_oids = Vec::new();
    let mut input_types = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let mode = modes.map_or("i", |modes| text(&modes[index]));
        assert!(matches!(mode, "i" | "o" | "b" | "t"));
        if matches!(mode, "i" | "b") {
            input_oids.push(argument["oid"].clone());
            input_types.push(type_name(&argument["identity"]));
        }
    }
    assert_eq!(raw["proargtypes"], Value::Array(input_oids));
    assert_eq!(
        raw["pronargs"].as_u64(),
        Some(u64::try_from(input_types.len()).unwrap())
    );
    input_types
}

pub fn validate(routine: &Value) {
    keys(
        routine,
        &[
            "raw",
            "identity",
            "address",
            "owner",
            "language",
            "kind",
            "support_oid",
            "arguments",
            "identity_arguments",
            "result",
            "argument_types",
            "return_type",
            "definition",
            "aggregate",
        ],
    );
    keys(&routine["raw"], &RAW_FIELDS);
    let raw = &routine["raw"];
    oid(&raw["oid"]);
    for field in ["pronamespace", "proowner", "prolang", "prorettype"] {
        oid(&raw[field]);
    }
    // Canonical source defines functions, no procedures, aggregates or windows.
    // Capture supports those kinds; this source oracle denies any unexpected kind.
    assert_eq!(routine["kind"], "f");
    assert_eq!(raw["prokind"], "f");
    assert!(routine["aggregate"].is_null());
    assert_eq!(raw["provariadic"], "0");
    assert_eq!(raw["prosupport"], "-"); // regproc zero renders as "-", not OID "0".
    assert_eq!(routine["support_oid"], "0");
    assert!(raw["protrftypes"].is_null());
    assert!(
        raw["prosqlbody"].is_null(),
        "SQL-standard bodies need a separate semantic witness"
    );
    // Original migrations explicitly assign some functions to authz_owner.
    // assert_old preserves owner/proowner exactly; assert_reference compares
    // the captured role identity with the independently qualified fresh head.
    assert!(!text(&routine["owner"]).is_empty());
    assert_eq!(routine["identity"]["schema"], "public");
    assert_eq!(routine["language"]["oid"], raw["prolang"]);
    type_name(&routine["return_type"]);
    input_types(routine);
    for field in ["arguments", "identity_arguments", "result", "definition"] {
        text(&routine[field]);
    }
    let defaults = raw["pronargdefaults"].as_u64().expect("default count");
    assert_eq!(raw["proargdefaults"].is_null(), defaults == 0);
}

pub fn normalized(routine: &Value, language: &Value) -> Value {
    validate(routine);
    let mut value = routine.clone();
    let mut normalized_raw = object(&routine["raw"]).clone();
    normalized_raw.insert("oid".into(), routine["identity"].clone());
    normalized_raw.insert("pronamespace".into(), routine["identity"]["schema"].clone());
    normalized_raw.insert("proowner".into(), routine["owner"].clone());
    normalized_raw.insert("prolang".into(), language.clone());
    normalized_raw.insert("prorettype".into(), routine["return_type"].clone());
    normalized_raw.insert(
        "proargtypes".into(),
        serde_json::to_value(input_types(routine)).unwrap(),
    );
    let argument_values: Vec<_> = arguments(routine)
        .iter()
        .map(|argument| {
            let mut argument = object(argument).clone();
            argument.insert("oid".into(), argument["identity"].clone());
            Value::Object(argument)
        })
        .collect();
    if !normalized_raw["proallargtypes"].is_null() {
        normalized_raw.insert(
            "proallargtypes".into(),
            Value::Array(
                argument_values
                    .iter()
                    .map(|argument| argument["identity"].clone())
                    .collect(),
            ),
        );
    }
    value["raw"] = Value::Object(normalized_raw);
    value["language"] = language.clone();
    if !value["argument_types"].is_null() {
        value["argument_types"] = Value::Array(argument_values);
    }
    // proargdefaults remains exact raw pg_node_tree. Both server-rendered
    // arguments and full definition must also match. No expression/OID rewrite.
    value
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
