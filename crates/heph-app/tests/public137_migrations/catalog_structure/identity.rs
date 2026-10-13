use super::super::catalog::Snapshot;
use super::{Inventory, Structure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn text(value: &Value) -> &str {
    value.as_str().expect("captured text")
}

pub(super) fn oid(value: &Value) -> &str {
    let id = text(value);
    assert!(!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()));
    assert!(id == "0" || !id.starts_with('0'));
    id.parse::<u32>().expect("OID fits u32");
    id
}

pub(super) fn rows<'a>(snapshot: &'a Snapshot, domain: &str) -> &'a [Value] {
    snapshot.domains()[domain].json().as_array().unwrap()
}

pub(super) fn keys(value: &Value, fields: &[&str]) {
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        fields.iter().copied().collect()
    );
}

pub(super) fn object(value: &Value) {
    keys(value, &["type", "schema", "name", "identity"]);
    assert!(value["type"].is_string() && value["identity"].is_string());
    assert!(value["schema"].is_string() || value["schema"].is_null());
    assert!(value["name"].is_string() || value["name"].is_null());
}

pub(super) fn ids(inventory: &Inventory<'_>, field: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for (key, record) in inventory {
        let id = oid(&record.value["raw"][field]);
        assert_ne!(id, "0");
        assert!(
            result.insert(id.to_owned(), key.clone()).is_none(),
            "duplicate typed OID"
        );
    }
    result
}

pub(super) fn owner(snapshot: &Snapshot, raw: &Value, helper: &Value) {
    let id = oid(raw);
    let matches: Vec<_> = rows(snapshot, "roles")
        .iter()
        .filter(|role| role["oid"] == id)
        .collect();
    assert_eq!(matches.len(), 1, "owner role identity witness");
    assert_eq!(matches[0]["rolname"], *helper);
}

pub(super) fn namespace(snapshot: &Snapshot, raw: &Value, name: &str) {
    if name == "public" {
        let matches: Vec<_> = rows(snapshot, "namespaces")
            .iter()
            .filter(|namespace| namespace["raw"]["nspname"] == name)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["raw"]["oid"], *raw);
        owner(
            snapshot,
            &matches[0]["raw"]["nspowner"],
            &matches[0]["owner"],
        );
    } else {
        assert_eq!(name, "pg_toast");
        assert_eq!(oid(raw), "99", "qualified PG17 built-in namespace");
    }
}

pub(super) fn class_link(structure: &Structure<'_>, raw: &Value, helper: &Value) -> Value {
    let id = oid(raw);
    if id == "0" {
        assert!(helper.is_null());
        return json!("0");
    }
    captured_link(&structure.relations, &structure.classes, raw, helper)
}

pub(super) fn captured_link(
    inventory: &Inventory<'_>,
    ids: &BTreeMap<String, String>,
    raw: &Value,
    helper: &Value,
) -> Value {
    let key = &ids[oid(raw)];
    assert_eq!(
        inventory[key].value["identity"], *helper,
        "typed identity link witness"
    );
    json!(key)
}

pub(super) fn type_link(structure: &Structure<'_>, raw: &Value, helper: &Value) -> Value {
    let id = oid(raw);
    if id == "0" {
        assert!(helper.is_null());
        return json!("0");
    }
    object(helper);
    structure.type_ids.get(id).map_or_else(
        || {
            assert!(
                id.parse::<u32>().unwrap() < 16384,
                "unresolved non-built-in type"
            );
            assert_eq!(helper["schema"], "pg_catalog");
            json!({"builtin_oid": id, "identity": helper})
        },
        |key| {
            let binding = captured_link(&structure.types, &structure.type_ids, raw, helper);
            assert_eq!(binding, *key);
            json!({"type": binding})
        },
    )
}

pub(super) fn generated_name(parent_oid: &str, index: bool) -> String {
    oid(&json!(parent_oid));
    assert_ne!(parent_oid, "0");
    format!("pg_toast_{parent_oid}{}", if index { "_index" } else { "" })
}

pub(super) fn toast_identity(value: &Value, name: &str, stable: &str) -> Value {
    object(value);
    assert_eq!(value["schema"], "pg_toast");
    assert_eq!(value["name"], name);
    assert_eq!(value["identity"], format!("pg_toast.{name}"));
    let mut result = value.clone();
    result["name"] = json!(stable);
    result["identity"] = json!(stable);
    result
}

pub(super) fn toast_column_identity(
    value: &Value,
    table: &str,
    column: &str,
    stable: &str,
) -> Value {
    object(value);
    assert_eq!(value["schema"], "pg_toast");
    // pg_identify_object's name is the containing pg_class name even for subIDs.
    assert_eq!(value["name"], table);
    assert_eq!(value["identity"], format!("pg_toast.{table}.{column}"));
    let mut result = value.clone();
    result["name"] = json!(stable);
    result["identity"] = json!(format!("{stable}.{column}"));
    result
}

pub(super) fn vector(value: &Value) -> Vec<&Value> {
    value.as_array().expect("ordered vector").iter().collect()
}
