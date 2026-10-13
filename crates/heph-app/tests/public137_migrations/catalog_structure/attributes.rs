//! Complete attributes excluded by the ordinary public-table user-column oracle.
use super::{Inventory, Structure, exact, identity, records, replace, semantic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const FIELDS: &[&str] = &[
    "attrelid",
    "attname",
    "atttypid",
    "attlen",
    "attnum",
    "attcacheoff",
    "atttypmod",
    "attndims",
    "attbyval",
    "attalign",
    "attstorage",
    "attcompression",
    "attnotnull",
    "atthasdef",
    "atthasmissing",
    "attidentity",
    "attgenerated",
    "attisdropped",
    "attislocal",
    "attinhcount",
    "attcollation",
    "attstattarget",
    "attacl",
    "attoptions",
    "attfdwoptions",
    "attmissingval",
];

fn number(raw: &Value) -> i16 {
    let number = i16::try_from(raw["attnum"].as_i64().expect("signed attribute number")).unwrap();
    assert_ne!(number, 0, "no zero attribute number");
    number
}

fn inventory<'a>(structure: &Structure<'a>) -> Inventory<'a> {
    let mut result = BTreeMap::new();
    for record in records(structure.snapshot, "columns") {
        let raw = &record.value["raw"];
        let relation = &structure.classes[identity::oid(&raw["attrelid"])];
        let table = structure.relations[relation].value;
        let number = number(raw);
        if table["namespace"] == "public" && table["raw"]["relkind"] == "r" && number > 0 {
            continue;
        }
        identity::keys(
            record.value,
            &[
                "raw",
                "relation",
                "column_identity",
                "formatted_type",
                "type",
                "collation",
            ],
        );
        identity::keys(raw, FIELDS);
        assert_eq!(record.value["relation"], table["identity"]);
        assert_eq!(
            raw["attisdropped"].as_bool().unwrap(),
            record.value["column_identity"].is_null()
        );
        assert!(
            result
                .insert(format!("{relation}@{number}"), record)
                .is_none(),
            "duplicate excluded attribute"
        );
    }
    result
}

fn normalized(structure: &Structure<'_>, record: super::Record<'_>) -> super::Semantic {
    let value = record.value;
    let raw = &value["raw"];
    let key = &structure.classes[identity::oid(&raw["attrelid"])];
    let relation = structure.relations[key].value;
    let mut result = semantic(record);
    replace(
        &mut result,
        "raw.attrelid",
        &identity::class_link(structure, &raw["attrelid"], &value["relation"]),
    );
    replace(&mut result, "relation", &json!(key));
    replace(
        &mut result,
        "raw.atttypid",
        &identity::type_link(structure, &raw["atttypid"], &value["type"]),
    );
    if identity::oid(&raw["attcollation"]) == "0" {
        assert!(value["collation"].is_null());
    } else {
        let bindings: Vec<_> = identity::rows(structure.snapshot, "collations")
            .iter()
            .filter(|entry| entry["raw"]["oid"] == raw["attcollation"])
            .collect();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0]["identity"], value["collation"]);
        assert!(identity::oid(&raw["attcollation"]).parse::<u32>().unwrap() < 16384);
        assert_eq!(value["collation"]["schema"], "pg_catalog");
    }
    if !value["column_identity"].is_null() {
        identity::object(&value["column_identity"]);
        if relation["namespace"] == "pg_toast" {
            replace(
                &mut result,
                "column_identity",
                &identity::toast_column_identity(
                    &value["column_identity"],
                    identity::text(&relation["raw"]["relname"]),
                    identity::text(&raw["attname"]),
                    key,
                ),
            );
        } else {
            assert_eq!(value["column_identity"]["schema"], "public");
            assert_eq!(value["column_identity"]["name"], relation["raw"]["relname"]);
            // The fresh full identity supplies PostgreSQL's identifier spelling.
            // Keep its complete value and lossless text in the canonical comparison;
            // relation/slot/name bindings and every raw field remain checked there.
        }
    }
    result
}

pub(super) fn compare(
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    fresh: &Structure<'_>,
    added: &BTreeSet<String>,
    removed: &str,
    replaced_children: &BTreeSet<String>,
) {
    if !replaced_children.is_empty() {
        assert_eq!(
            replaced_children,
            &BTreeSet::from([
                "toast(public.agent_instance_volume_leases)".to_owned(),
                "toast_index(public.agent_instance_volume_leases)".to_owned(),
            ]),
            "only separately qualified source108 rewrite children"
        );
    }
    let original = inventory(old);
    let current = inventory(upgraded);
    let canonical = inventory(fresh);
    let removed_prefix = format!("{removed}@");
    let mut expected = BTreeSet::new();
    for (key, record) in &original {
        if !key.starts_with(&removed_prefix) {
            let relation = &old.classes[identity::oid(&record.value["raw"]["attrelid"])];
            if replaced_children.contains(relation) {
                // Caller must first qualify the source108 physical rewrite.
                // All non-link fields remain lossless in this old→new comparison.
                assert_shape(
                    &normalized(old, *record),
                    &normalized(upgraded, current[key]),
                );
            } else {
                exact(*record, current[key], "original excluded attribute changed");
            }
            expected.insert(key.clone());
        }
    }
    for (key, record) in &canonical {
        let relation = &fresh.classes[identity::oid(&record.value["raw"]["attrelid"])];
        if added.contains(relation) {
            assert!(expected.insert(key.clone()));
        }
    }
    assert_eq!(
        canonical.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "only source-admitted excluded attributes"
    );
    assert_eq!(
        current.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "complete excluded attribute inventory"
    );
    for (key, record) in &current {
        assert_shape(
            &normalized(upgraded, *record),
            &normalized(fresh, canonical[key]),
        );
    }
}

fn assert_shape(actual: &super::Semantic, expected: &super::Semantic) {
    assert_eq!(
        actual, expected,
        "complete canonical excluded attribute shape"
    );
}

#[cfg(test)]
#[path = "attribute_tests.rs"]
mod tests;
