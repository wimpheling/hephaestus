use super::{Inventory, Semantic, Structure, exact, identity, records, replace, semantic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const FIELDS: &[&str] = &[
    "indexrelid",
    "indrelid",
    "indnatts",
    "indnkeyatts",
    "indisunique",
    "indnullsnotdistinct",
    "indisprimary",
    "indisexclusion",
    "indimmediate",
    "indisclustered",
    "indisvalid",
    "indcheckxmin",
    "indisready",
    "indislive",
    "indisreplident",
    "indkey",
    "indcollation",
    "indclass",
    "indoption",
    "indexprs",
    "indpred",
];

pub(super) fn inventory<'a>(structure: &Structure<'a>) -> Inventory<'a> {
    let mut result = BTreeMap::new();
    for record in records(structure.snapshot, "indexes") {
        identity::keys(&record.value["raw"], FIELDS);
        identity::keys(
            record.value,
            &["raw", "identity", "table", "definition", "columns"],
        );
        let key = &structure.classes[identity::oid(&record.value["raw"]["indexrelid"])];
        assert_eq!(structure.relations[key].value["raw"]["relkind"], "i");
        assert_eq!(
            record.value["identity"],
            structure.relations[key].value["identity"]
        );
        assert!(
            result.insert(key.clone(), record).is_none(),
            "duplicate index"
        );
    }
    let declared: BTreeSet<_> = structure
        .relations
        .iter()
        .filter(|(_, relation)| relation.value["raw"]["relkind"] == "i")
        .map(|(key, _)| key.clone())
        .collect();
    assert_eq!(
        result.keys().cloned().collect::<BTreeSet<_>>(),
        declared,
        "complete index inventory"
    );
    result
}

pub(super) fn compare(
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    fresh: &Structure<'_>,
    additions: &BTreeSet<String>,
    removed: &str,
    replaced_children: &BTreeSet<String>,
) {
    let original = inventory(old);
    let current = inventory(upgraded);
    let canonical = inventory(fresh);
    assert!(original.contains_key(removed));
    for (key, record) in &original {
        if replaced_children.contains(key) {
            assert_eq!(
                normalized(old, key, *record),
                normalized(upgraded, key, current[key]),
                "complete rewritten TOAST index preservation {key}"
            );
        } else if key != removed {
            exact(*record, current[key], key);
        }
    }
    for (key, record) in &current {
        if additions.contains(key) {
            assert!(
                record.value["raw"]["indexprs"].is_null(),
                "no declared new expression index"
            );
            if key == "public.one_active_run_volume_slot" {
                assert_eq!(record.value["raw"]["indisunique"], true);
                assert_eq!(record.value["raw"]["indisprimary"], false);
                assert!(!record.value["raw"]["indpred"].is_null());
                let names = column_names(upgraded, record.value);
                assert_eq!(names, ["run_id", "slot_key"]);
            } else {
                assert!(record.value["raw"]["indpred"].is_null());
            }
            assert!(
                !original
                    .values()
                    .any(|old| old.value["raw"]["indexrelid"] == record.value["raw"]["indexrelid"])
            );
        }
        assert_eq!(
            normalized(upgraded, key, *record),
            normalized(fresh, key, canonical[key]),
            "complete canonical index {key}, including validity and indcheckxmin"
        );
    }
}

fn column_names(structure: &Structure<'_>, index: &Value) -> Vec<String> {
    index["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let column = column(structure, &index["raw"]["indrelid"], &item["attnum"]);
            identity::text(&column["raw"]["attname"]).to_owned()
        })
        .collect()
}

fn column<'a>(structure: &Structure<'a>, relation: &Value, number: &Value) -> &'a Value {
    let matches: Vec<_> = identity::rows(structure.snapshot, "columns")
        .iter()
        .filter(|column| {
            column["raw"]["attrelid"] == *relation && column["raw"]["attnum"] == *number
        })
        .collect();
    assert_eq!(matches.len(), 1, "indexed attribute witness");
    assert_eq!(matches[0]["raw"]["attisdropped"], false);
    matches[0]
}

pub(super) fn ordered_columns(raw: &Value, helpers: &Value) {
    let numbers = identity::vector(&raw["indkey"]);
    let observed = helpers.as_array().unwrap();
    let count = raw["indnatts"].as_u64().unwrap();
    let keys = raw["indnkeyatts"].as_u64().unwrap();
    assert!(keys > 0 && keys <= count);
    assert_eq!(u64::try_from(numbers.len()).unwrap(), count);
    assert_eq!(numbers.len(), observed.len());
    for (position, (number, helper)) in numbers.iter().zip(observed).enumerate() {
        identity::keys(helper, &["position", "attnum", "identity"]);
        assert_eq!(helper["position"], position + 1);
        assert_eq!(helper["attnum"], **number);
        assert!(
            number
                .as_i64()
                .is_some_and(|number| i16::try_from(number).is_ok())
        );
        if number.as_i64() == Some(0) {
            assert!(helper["identity"].is_null());
        } else {
            identity::object(&helper["identity"]);
        }
    }
    for field in ["indclass", "indcollation", "indoption"] {
        assert_eq!(
            u64::try_from(identity::vector(&raw[field]).len()).unwrap(),
            keys
        );
    }
}

fn qualified_vector(structure: &Structure<'_>, values: &Value, domain: &str) -> Value {
    json!(
        identity::vector(values)
            .iter()
            .map(|value| {
                let id = identity::oid(value);
                if id == "0" {
                    assert_eq!(domain, "collations");
                    return json!("0");
                }
                let witnesses: Vec<_> = identity::rows(structure.snapshot, domain)
                    .iter()
                    .filter(|witness| witness["raw"]["oid"] == id)
                    .collect();
                assert_eq!(witnesses.len(), 1, "referenced {domain} witness");
                let helper = &witnesses[0]["identity"];
                identity::object(helper);
                assert!(
                    id.parse::<u32>().unwrap() < 16384,
                    "source has no non-built-in index class/collation"
                );
                assert_eq!(helper["schema"], "pg_catalog");
                json!({"builtin_oid": id, "identity": helper})
            })
            .collect::<Vec<_>>()
    )
}

pub(super) fn toast_definition(name: &str, table: &str) -> String {
    format!("CREATE UNIQUE INDEX {name} ON pg_toast.{table} USING btree (chunk_id, chunk_seq)")
}

pub(super) fn normalized(
    structure: &Structure<'_>,
    key: &str,
    record: super::Record<'_>,
) -> Semantic {
    let value = record.value;
    let raw = &value["raw"];
    ordered_columns(raw, &value["columns"]);
    let table = &structure.classes[identity::oid(&raw["indrelid"])];
    assert_eq!(value["table"], structure.relations[table].value["identity"]);
    let mut result = semantic(record);
    replace(&mut result, "raw.indexrelid", &json!(key));
    replace(
        &mut result,
        "raw.indrelid",
        &identity::class_link(structure, &raw["indrelid"], &value["table"]),
    );
    for (field, domain) in [
        ("indclass", "operator_classes"),
        ("indcollation", "collations"),
    ] {
        replace(
            &mut result,
            &format!("raw.{field}"),
            &qualified_vector(structure, &raw[field], domain),
        );
    }
    let mut columns = value["columns"].clone();
    for helper in columns.as_array_mut().unwrap() {
        if helper["attnum"] == 0 {
            continue;
        }
        let attribute = column(structure, &raw["indrelid"], &helper["attnum"]);
        assert_eq!(attribute["relation"], value["table"]);
        assert_eq!(attribute["column_identity"], helper["identity"]);
        if table.starts_with("toast(") {
            let name = identity::text(&structure.relations[table].value["raw"]["relname"]);
            let attribute_name = identity::text(&attribute["raw"]["attname"]);
            helper["identity"] =
                identity::toast_column_identity(&helper["identity"], name, attribute_name, table);
        }
    }
    replace(&mut result, "columns", &columns);
    if key.starts_with("toast_index(") {
        let name = identity::text(&structure.relations[key].value["raw"]["relname"]);
        let table_name = identity::text(&structure.relations[table].value["raw"]["relname"]);
        let parent = key
            .strip_prefix("toast_index(")
            .unwrap()
            .strip_suffix(')')
            .unwrap();
        assert_eq!(
            table,
            &format!("toast({parent})"),
            "TOAST index reciprocal table link"
        );
        assert_eq!(value["definition"], toast_definition(name, table_name));
        replace(
            &mut result,
            "definition",
            &json!(toast_definition(key, table)),
        );
        replace(
            &mut result,
            "identity",
            &identity::toast_identity(&value["identity"], name, key),
        );
        replace(
            &mut result,
            "table",
            &identity::toast_identity(&value["table"], table_name, table),
        );
    }
    result
}
