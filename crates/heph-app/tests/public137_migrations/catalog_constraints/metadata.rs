use super::super::catalog::Snapshot;
use super::{Group, expression, identity, json_text};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Constraint<'a> {
    pub(super) key: String,
    pub(super) group: Group,
    pub(super) value: &'a Value,
    pub(super) text: &'a str,
}

const FIELDS: &[&str] = &[
    "oid",
    "conname",
    "connamespace",
    "contype",
    "condeferrable",
    "condeferred",
    "convalidated",
    "conrelid",
    "contypid",
    "conindid",
    "conparentid",
    "confrelid",
    "confupdtype",
    "confdeltype",
    "confmatchtype",
    "conislocal",
    "coninhcount",
    "connoinherit",
    "conkey",
    "confkey",
    "conpfeqop",
    "conppeqop",
    "conffeqop",
    "confdelsetcols",
    "conexclop",
    "conbin",
];

pub(super) fn constraints(snapshot: &Snapshot) -> BTreeMap<String, Constraint<'_>> {
    let relations = identity::relations(snapshot);
    let namespace = identity::public_namespace(snapshot);
    let capture = snapshot
        .domains()
        .get("constraints")
        .expect("constraint capture");
    let parsed = identity::rows(snapshot, "constraints");
    let texts = json_text::array_items(capture.text());
    assert_eq!(parsed.len(), texts.len());
    let mut result = BTreeMap::new();
    let mut oids = BTreeSet::new();
    for (value, text) in parsed.iter().zip(texts) {
        assert_eq!(&serde_json::from_str::<Value>(text).unwrap(), value);
        identity::keys(
            value,
            &[
                "raw",
                "identity",
                "relation",
                "foreign_relation",
                "domain",
                "definition",
                "columns",
                "foreign_columns",
            ],
        );
        identity::object(&value["identity"]);
        assert!(value["definition"].is_string());
        let raw = &value["raw"];
        validate_raw(raw);
        for field in FIELDS {
            json_text::field(json_text::field(text, "raw"), field);
        }
        assert!(oids.insert(identity::oid(&raw["oid"])));
        assert_eq!(raw["connamespace"], namespace["raw"]["oid"]);
        assert!(value["domain"].is_null());
        let relation = relations
            .get(identity::oid(&raw["conrelid"]))
            .expect("constraint relation captured");
        assert_eq!(value["relation"], relation["identity"]);
        assert_eq!(relation["namespace"], "public");
        assert_eq!(relation["owner"], "postgres");
        let kind = identity::text(&raw["contype"]).to_owned();
        let mut columns = identity::columns(snapshot, relation, &raw["conkey"], &value["columns"]);
        if kind == "c" {
            columns.sort();
        }
        let foreign = foreign(snapshot, &relations, value);
        supporting_index(snapshot, &relations, value);
        let table = identity::text(&relation["raw"]["relname"]).to_owned();
        let key = format!("public.{table}.{}", identity::text(&raw["conname"]));
        let group = Group {
            table,
            kind,
            columns,
            foreign_table: foreign.0,
            foreign_columns: foreign.1,
        };
        assert!(
            result
                .insert(
                    key.clone(),
                    Constraint {
                        key,
                        group,
                        value,
                        text
                    }
                )
                .is_none()
        );
    }
    result
}

fn validate_raw(raw: &Value) {
    identity::keys(raw, FIELDS);
    for field in [
        "oid",
        "connamespace",
        "conrelid",
        "contypid",
        "conindid",
        "conparentid",
        "confrelid",
    ] {
        identity::oid(&raw[field]);
    }
    assert_ne!(raw["oid"], "0");
    assert_ne!(raw["conrelid"], "0");
    assert_eq!(raw["contypid"], "0", "no source domain declaration");
    assert_eq!(raw["conparentid"], "0", "no source partition declaration");
    assert!(matches!(
        identity::text(&raw["contype"]),
        "c" | "p" | "u" | "f" | "t"
    ));
    assert!(!identity::text(&raw["conname"]).is_empty());
    for field in [
        "condeferrable",
        "condeferred",
        "convalidated",
        "conislocal",
        "connoinherit",
    ] {
        assert!(raw[field].is_boolean());
    }
    assert!(raw["coninhcount"].as_i64().is_some());
    for field in ["confupdtype", "confdeltype", "confmatchtype"] {
        assert_eq!(identity::text(&raw[field]).len(), 1);
    }
    for field in ["conpfeqop", "conppeqop", "conffeqop", "conexclop"] {
        if !raw[field].is_null() {
            for operator in raw[field].as_array().expect("operator OID array") {
                let operator: u32 = identity::oid(operator).parse().unwrap();
                // All finite source constraint operators are native builtins.
                assert!((1..16384).contains(&operator), "unsupported user operator");
            }
        }
    }
    if !raw["confdelsetcols"].is_null() {
        for number in raw["confdelsetcols"].as_array().unwrap() {
            assert!(
                number
                    .as_i64()
                    .is_some_and(|number| number > 0 && i16::try_from(number).is_ok())
            );
        }
    }
    assert!(raw["conbin"].is_null() || raw["conbin"].is_string());
    assert_eq!(raw["conbin"].is_string(), raw["contype"] == "c");
}

fn foreign(
    snapshot: &Snapshot,
    relations: &BTreeMap<&str, &Value>,
    value: &Value,
) -> (Option<String>, Vec<String>) {
    let raw = &value["raw"];
    if raw["confrelid"] == "0" {
        assert_ne!(raw["contype"], "f");
        assert!(value["foreign_relation"].is_null());
        assert!(raw["confkey"].is_null() && value["foreign_columns"].is_null());
        return (None, Vec::new());
    }
    assert_eq!(raw["contype"], "f");
    let relation = relations
        .get(identity::oid(&raw["confrelid"]))
        .expect("foreign relation captured");
    assert_eq!(relation["namespace"], "public");
    assert_eq!(relation["owner"], "postgres");
    assert_eq!(value["foreign_relation"], relation["identity"]);
    let columns = identity::columns(
        snapshot,
        relation,
        &raw["confkey"],
        &value["foreign_columns"],
    );
    (
        Some(identity::text(&relation["raw"]["relname"]).to_owned()),
        columns,
    )
}

fn supporting_index(snapshot: &Snapshot, relations: &BTreeMap<&str, &Value>, value: &Value) {
    let raw = &value["raw"];
    if raw["conindid"] == "0" {
        assert!(matches!(identity::text(&raw["contype"]), "c" | "t"));
        return;
    }
    assert!(matches!(identity::text(&raw["contype"]), "p" | "u" | "f"));
    let index = relations
        .get(identity::oid(&raw["conindid"]))
        .expect("supporting index captured");
    assert_eq!(index["raw"]["relkind"], "i");
    let matches: Vec<_> = identity::rows(snapshot, "indexes")
        .iter()
        .filter(|index| index["raw"]["indexrelid"] == raw["conindid"])
        .collect();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["identity"], index["identity"]);
    let parent = if raw["contype"] == "f" {
        "confrelid"
    } else {
        "conrelid"
    };
    assert_eq!(matches[0]["raw"]["indrelid"], raw[parent]);
}

pub(super) fn normalized(snapshot: &Snapshot, constraint: &Constraint<'_>) -> Value {
    let mut value = constraint.value.clone();
    let relations = identity::relations(snapshot);
    // Only explicit, identity-typed pg_constraint OIDs change representation.
    // All attribute numbers, operators, flags, node bytes and deparser text stay.
    value["raw"]["oid"] = value["identity"].clone();
    assert_eq!(
        value["raw"]["connamespace"],
        identity::public_namespace(snapshot)["raw"]["oid"]
    );
    value["raw"]["connamespace"] = json!({"namespace": "public"});
    for field in ["conrelid", "confrelid", "conindid"] {
        if value["raw"][field] != "0" {
            let relation = relations.get(identity::oid(&value["raw"][field])).unwrap();
            value["raw"][field] = relation["identity"].clone();
        }
    }
    if !value["raw"]["conbin"].is_null() {
        value["raw"]["conbin"] = expression::normalized(
            identity::text(&value["raw"]["conbin"]),
            &identity::helpers(snapshot),
        );
    }
    value
}
