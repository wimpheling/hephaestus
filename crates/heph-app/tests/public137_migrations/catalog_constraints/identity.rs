use super::super::catalog::Snapshot;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn rows<'a>(snapshot: &'a Snapshot, name: &str) -> &'a [Value] {
    snapshot
        .domains()
        .get(name)
        .expect("catalog domain")
        .json()
        .as_array()
        .expect("catalog array")
}

pub(super) fn text(value: &Value) -> &str {
    value.as_str().expect("captured text")
}

pub(super) fn oid(value: &Value) -> &str {
    let value = text(value);
    assert!(!value.is_empty());
    assert!(value.bytes().all(|byte| byte.is_ascii_digit()));
    assert!(value == "0" || !value.starts_with('0'));
    value.parse::<u32>().expect("OID fits u32");
    value
}

pub(super) fn keys(value: &Value, expected: &[&str]) {
    let actual: BTreeSet<_> = value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(actual, expected.iter().copied().collect());
}

pub(super) fn object(value: &Value) {
    keys(value, &["type", "schema", "name", "identity"]);
    assert!(value["type"].is_string());
    assert!(value["schema"].is_string() || value["schema"].is_null());
    assert!(value["name"].is_string() || value["name"].is_null());
    assert!(value["identity"].is_string());
}

pub(super) fn relations(snapshot: &Snapshot) -> BTreeMap<&str, &Value> {
    let mut result = BTreeMap::new();
    for relation in rows(snapshot, "relations") {
        object(&relation["identity"]);
        let id = oid(&relation["raw"]["oid"]);
        assert_ne!(id, "0");
        assert!(result.insert(id, relation).is_none());
    }
    result
}

pub(super) fn columns(
    snapshot: &Snapshot,
    relation: &Value,
    raw: &Value,
    helper: &Value,
) -> Vec<String> {
    if raw.is_null() {
        assert!(helper.is_null());
        return Vec::new();
    }
    let numbers = raw.as_array().expect("constraint attribute array");
    let observed = helper.as_array().expect("ordered column helper");
    assert_eq!(numbers.len(), observed.len());
    let mut result = Vec::new();
    for (position, (number, item)) in numbers.iter().zip(observed).enumerate() {
        keys(item, &["position", "attnum", "identity"]);
        let number = number.as_i64().expect("attribute number");
        assert!(number > 0 && i16::try_from(number).is_ok());
        assert_eq!(
            item["position"].as_u64().unwrap(),
            u64::try_from(position + 1).unwrap()
        );
        assert_eq!(item["attnum"].as_i64().unwrap(), number);
        object(&item["identity"]);
        let matches: Vec<_> = rows(snapshot, "columns")
            .iter()
            .filter(|column| {
                column["raw"]["attrelid"] == relation["raw"]["oid"]
                    && column["raw"]["attnum"].as_i64() == Some(number)
            })
            .collect();
        assert_eq!(matches.len(), 1);
        let column = matches[0];
        assert_eq!(column["raw"]["attisdropped"], false);
        assert_eq!(column["column_identity"], item["identity"]);
        result.push(text(&column["raw"]["attname"]).to_owned());
    }
    assert_eq!(result.iter().collect::<BTreeSet<_>>().len(), result.len());
    result
}

pub(super) fn public_namespace(snapshot: &Snapshot) -> &Value {
    let namespaces = rows(snapshot, "namespaces");
    assert_eq!(namespaces.len(), 1);
    let namespace = &namespaces[0];
    assert_eq!(namespace["raw"]["nspname"], "public");
    // The template's schema owner is independent of created table/routine owners;
    // PostgreSQL17 initially uses pg_database_owner. The complete namespace
    // owner/OID/ACL capture stays exact in catalog_stability upgrade/replay checks.
    assert!(!text(&namespace["owner"]).is_empty());
    assert_ne!(oid(&namespace["raw"]["oid"]), "0");
    namespace
}

pub(super) fn helpers(snapshot: &Snapshot) -> BTreeMap<String, Value> {
    let allowed = [
        "private_volume_guest_path_is_valid",
        "run_cleanup_identifier_is_valid",
    ];
    let mut result = BTreeMap::new();
    let mut found = BTreeSet::new();
    for routine in rows(snapshot, "routines") {
        let raw = &routine["raw"];
        if !allowed.contains(&text(&raw["proname"])) {
            continue;
        }
        object(&routine["identity"]);
        assert_eq!(routine["identity"]["schema"], "public");
        assert_eq!(routine["owner"], "postgres");
        assert_eq!(
            raw["pronamespace"],
            public_namespace(snapshot)["raw"]["oid"]
        );
        assert!(oid(&raw["oid"]).parse::<u32>().unwrap() >= 16384);
        object(&routine["return_type"]);
        assert_eq!(routine["return_type"]["schema"], "pg_catalog");
        assert_eq!(routine["return_type"]["name"], "bool");
        assert_eq!(raw["pronargs"], 1);
        assert_eq!(raw["proargtypes"], serde_json::json!(["25"]));
        assert_eq!(raw["prorettype"], "16");
        assert_eq!(raw["prokind"], "f");
        let args = routine["argument_types"].as_array().unwrap();
        assert_eq!(args.len(), 1);
        assert_eq!(args[0]["position"], 1);
        assert_eq!(oid(&args[0]["oid"]), "25");
        assert_eq!(args[0]["identity"]["schema"], "pg_catalog");
        assert_eq!(args[0]["identity"]["name"], "text");
        assert!(found.insert(text(&raw["proname"]).to_owned()));
        let signature = format!("public.{}(pg_catalog.text)", text(&raw["proname"]));
        let binding = serde_json::json!({"signature": signature, "identity": routine["identity"]});
        assert!(
            result
                .insert(oid(&raw["oid"]).to_owned(), binding)
                .is_none()
        );
    }
    assert_eq!(found, allowed.into_iter().map(str::to_owned).collect());
    result
}

#[cfg(test)]
#[path = "identity/tests.rs"]
mod tests;
