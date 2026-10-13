use super::{
    Inventory, Record, Semantic, Structure, builtin, compare_domain, extension, identity, json,
    records, replace, routine, semantic, signed,
};
use std::collections::BTreeSet;

pub(super) fn compare(
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    fresh: &Structure<'_>,
    old_extensions: &Inventory<'_>,
    current_extensions: &Inventory<'_>,
    fresh_extensions: &Inventory<'_>,
) {
    let original = inventory(old, old_extensions);
    let current = inventory(upgraded, current_extensions);
    let reference = inventory(fresh, fresh_extensions);
    let native: Vec<_> = reference
        .iter()
        .filter(|(_, record)| {
            record.value["raw"]["classoid"] == "3079"
                && record.value["identity"]["name"] == "pgcrypto"
        })
        .collect();
    assert_eq!(native.len(), 1, "one native control-file extension comment");
    native_comment(native[0].1.value);
    let expected = original
        .keys()
        .cloned()
        .chain([native[0].0.clone()])
        .collect();
    // No new routine, table, column, type or operator comment is admitted.
    compare_domain(
        &original,
        &current,
        &reference,
        &expected,
        |record| normalized(upgraded, current_extensions, record),
        |record| normalized(fresh, fresh_extensions, record),
    );
}

fn inventory<'a>(structure: &Structure<'a>, extensions: &Inventory<'_>) -> Inventory<'a> {
    let mut result = Inventory::new();
    let mut addresses = BTreeSet::new();
    for record in records(structure.snapshot, "descriptions") {
        identity::keys(record.value, &["raw", "identity"]);
        let raw = &record.value["raw"];
        identity::keys(raw, &["objoid", "classoid", "objsubid", "description"]);
        identity::oid(&raw["objoid"]);
        let class = identity::oid(&raw["classoid"]);
        builtin(&raw["classoid"]);
        let number = signed(&raw["objsubid"]);
        assert!(raw["description"].is_string());
        identity::object(&record.value["identity"]);
        assert!(addresses.insert((class, identity::oid(&raw["objoid"]), number)));
        // Validate every typed link even before canonical comparison.
        binding(structure, extensions, record.value);
        let key = format!("{class}/{number}/{}", record.value["identity"]);
        assert!(
            result.insert(key, record).is_none(),
            "ambiguous comment identity"
        );
    }
    result
}

fn binding(
    structure: &Structure<'_>,
    extensions: &Inventory<'_>,
    entry: &serde_json::Value,
) -> serde_json::Value {
    let raw = &entry["raw"];
    let id = identity::oid(&raw["objoid"]);
    let helper = &entry["identity"];
    let number = signed(&raw["objsubid"]);
    match identity::oid(&raw["classoid"]) {
        "3079" => {
            assert_eq!(number, 0);
            let name = extension(extensions, &raw["objoid"]);
            assert_eq!(
                helper,
                &json!({"type":"extension", "schema":null, "name":name, "identity":name})
            );
            if id.parse::<u32>().unwrap() >= 16384 {
                return json!(name);
            }
        }
        "1259" if structure.classes.contains_key(id) => {
            let key = &structure.classes[id];
            if number == 0 {
                assert_eq!(*helper, structure.relations[key].value["identity"]);
            } else {
                let columns: Vec<_> = identity::rows(structure.snapshot, "columns")
                    .iter()
                    .filter(|column| {
                        column["raw"]["attrelid"] == id && column["raw"]["attnum"] == number
                    })
                    .collect();
                assert_eq!(columns.len(), 1, "signed column comment address witness");
                assert_eq!(*helper, columns[0]["column_identity"]);
            }
            return json!(key);
        }
        "1255" if id.parse::<u32>().unwrap() >= 16384 => {
            assert_eq!(number, 0);
            routine(structure, &raw["objoid"], helper);
            return helper.clone();
        }
        "1247" if structure.type_ids.contains_key(id) => {
            assert_eq!(number, 0);
            return identity::captured_link(
                &structure.types,
                &structure.type_ids,
                &raw["objoid"],
                helper,
            );
        }
        _ => {}
    }
    builtin(&raw["objoid"]);
    raw["objoid"].clone()
}

fn normalized(
    structure: &Structure<'_>,
    extensions: &Inventory<'_>,
    record: Record<'_>,
) -> Semantic {
    let mut result = semantic(record);
    replace(
        &mut result,
        "raw.objoid",
        &binding(structure, extensions, record.value),
    );
    result
}

pub(super) fn native_comment(entry: &serde_json::Value) {
    assert_eq!(entry["raw"]["classoid"], "3079");
    assert_eq!(signed(&entry["raw"]["objsubid"]), 0);
    assert_eq!(entry["raw"]["description"], "cryptographic functions");
    assert_eq!(
        entry["identity"],
        json!({"type":"extension", "schema":null, "name":"pgcrypto", "identity":"pgcrypto"})
    );
}
