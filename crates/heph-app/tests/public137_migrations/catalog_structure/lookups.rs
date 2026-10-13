//! Complete index-used lookup closure plus preserved public lookup objects.
use super::{Inventory, Structure, exact, identity, records, replace, semantic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const CLASS_FIELDS: &[&str] = &[
    "oid",
    "opcmethod",
    "opcname",
    "opcnamespace",
    "opcowner",
    "opcfamily",
    "opcintype",
    "opcdefault",
    "opckeytype",
];
const FAMILY_FIELDS: &[&str] = &["oid", "opfmethod", "opfname", "opfnamespace", "opfowner"];

// namespaces.sql captures the complete public row only. PG17's
// pg_namespace.dat fixes PG_CATALOG_NAMESPACE at 11; its actual lookup field
// remains lossless in old preservation and complete canonical comparisons.
fn namespace(raw: &Value, schema: &str, capture: &[Value]) {
    assert_eq!(capture.len(), 1, "complete public-only namespace capture");
    assert_eq!(capture[0]["raw"]["nspname"], "public");
    match schema {
        "public" => assert_eq!(*raw, capture[0]["raw"]["oid"]),
        "pg_catalog" => assert_eq!(identity::oid(raw), "11", "qualified PG17 builtin namespace"),
        other => panic!("undeclared lookup namespace {other}"),
    }
}

fn inventory<'a>(structure: &Structure<'a>, family: bool) -> Inventory<'a> {
    let domain = if family {
        "operator_families"
    } else {
        "operator_classes"
    };
    let prefix = if family { "opf" } else { "opc" };
    let mut result = BTreeMap::new();
    for record in records(structure.snapshot, domain) {
        let value = record.value;
        identity::keys(value, &["raw", "identity", "owner"]);
        let raw = &value["raw"];
        identity::keys(raw, if family { FAMILY_FIELDS } else { CLASS_FIELDS });
        identity::object(&value["identity"]);
        assert_eq!(
            value["identity"]["type"],
            if family {
                "operator family"
            } else {
                "operator class"
            }
        );
        let schema = identity::text(&value["identity"]["schema"]);
        assert!(matches!(schema, "public" | "pg_catalog"));
        assert_eq!(value["identity"]["name"], raw[format!("{prefix}name")]);
        namespace(
            &raw[format!("{prefix}namespace")],
            schema,
            identity::rows(structure.snapshot, "namespaces"),
        );
        identity::owner(
            structure.snapshot,
            &raw[format!("{prefix}owner")],
            &value["owner"],
        );
        assert_eq!(
            identity::oid(&raw[format!("{prefix}method")]),
            "403",
            "declared btree lookup method"
        );
        if schema == "pg_catalog" {
            assert!(identity::oid(&raw["oid"]).parse::<u32>().unwrap() < 16384);
        }
        let key = value["identity"].to_string();
        assert!(
            result.insert(key, record).is_none(),
            "duplicate lookup identity"
        );
    }
    assert_eq!(identity::ids(&result, "oid").len(), result.len());
    result
}

fn public_keys(inventory: &Inventory<'_>) -> BTreeSet<String> {
    inventory
        .iter()
        .filter(|(_, row)| row.value["identity"]["schema"] == "public")
        .map(|(key, _)| key.clone())
        .collect()
}

fn used_classes<'a>(structure: &Structure<'_>, classes: &Inventory<'a>) -> Inventory<'a> {
    let ids = identity::ids(classes, "oid");
    let mut used = BTreeMap::new();
    for index in records(structure.snapshot, "indexes") {
        // Parent already checks this full source-admitted index inventory.
        assert!(
            structure
                .classes
                .contains_key(identity::oid(&index.value["raw"]["indexrelid"]))
        );
        for id in identity::vector(&index.value["raw"]["indclass"]) {
            let key = &ids[identity::oid(id)];
            used.insert(key.clone(), classes[key]);
        }
    }
    used
}

fn family_closure(classes: &Inventory<'_>, families: &Inventory<'_>) -> BTreeSet<String> {
    let ids = identity::ids(families, "oid");
    let mut closure = public_keys(families);
    for class in classes.values() {
        closure.insert(ids[identity::oid(&class.value["raw"]["opcfamily"])].clone());
    }
    closure
}

fn normalized(
    structure: &Structure<'_>,
    record: super::Record<'_>,
    families: &Inventory<'_>,
    family: bool,
) -> super::Semantic {
    let mut result = semantic(record);
    let raw = &record.value["raw"];
    if record.value["identity"]["schema"] == "public" {
        replace(&mut result, "raw.oid", &record.value["identity"]);
    }
    if !family {
        let ids = identity::ids(families, "oid");
        let family_id = identity::oid(&raw["opcfamily"]);
        if let Some(key) = ids.get(family_id) {
            let binding = families[key];
            if binding.value["identity"]["schema"] == "public" {
                replace(&mut result, "raw.opcfamily", &binding.value["identity"]);
            }
        } else {
            // The query omits unused builtin families of public classes.
            assert!(family_id.parse::<u32>().unwrap() < 16384);
        }
        for field in ["opcintype", "opckeytype"] {
            let id = identity::oid(&raw[field]);
            if let Some(key) = structure.type_ids.get(id) {
                replace(&mut result, &format!("raw.{field}"), &json!({"type": key}));
            } else {
                assert!(id.parse::<u32>().unwrap() < 16384, "unresolved lookup type");
            }
        }
    }
    result
}

pub(super) fn compare(old: &Structure<'_>, upgraded: &Structure<'_>, fresh: &Structure<'_>) {
    let mut sets = Vec::new();
    for structure in [old, upgraded, fresh] {
        let classes = inventory(structure, false);
        let families = inventory(structure, true);
        let used = used_classes(structure, &classes);
        let class_closure: BTreeSet<_> = public_keys(&classes)
            .into_iter()
            .chain(used.keys().cloned())
            .collect();
        assert_closure(&classes, &class_closure);
        assert_closure(&families, &family_closure(&used, &families));
        sets.push((classes, families));
    }
    for family in [false, true] {
        let original = if family { &sets[0].1 } else { &sets[0].0 };
        let current = if family { &sets[1].1 } else { &sets[1].0 };
        let canonical = if family { &sets[2].1 } else { &sets[2].0 };
        assert_eq!(
            public_keys(original),
            public_keys(current),
            "no source public lookup creation"
        );
        assert_eq!(public_keys(original), public_keys(canonical));
        assert_eq!(
            current.keys().collect::<Vec<_>>(),
            canonical.keys().collect::<Vec<_>>(),
            "canonical complete lookup closure"
        );
        for (key, row) in original {
            exact(
                *row,
                current[key],
                "old lookup object changed or left capture scope",
            );
        }
        for (key, row) in current {
            assert_shape(
                &normalized(upgraded, *row, &sets[1].1, family),
                &normalized(fresh, canonical[key], &sets[2].1, family),
            );
        }
    }
}

fn assert_closure(inventory: &Inventory<'_>, expected: &BTreeSet<String>) {
    assert_eq!(
        &inventory.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "complete lookup closure"
    );
}

fn assert_shape(actual: &super::Semantic, expected: &super::Semantic) {
    assert_eq!(actual, expected, "complete canonical lookup shape");
}

#[cfg(test)]
#[path = "lookup_tests.rs"]
mod tests;
