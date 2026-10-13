use super::super::catalog::Snapshot;
use super::{Inventory, Semantic, Structure, exact, identity, records, replace, semantic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const HANDLERS: &[&str] = &[
    "typanalyze",
    "typinput",
    "typmodin",
    "typmodout",
    "typoutput",
    "typreceive",
    "typsend",
    "typsubscript",
];

const FIELDS: &[&str] = &[
    "oid",
    "typname",
    "typnamespace",
    "typowner",
    "typlen",
    "typbyval",
    "typtype",
    "typcategory",
    "typispreferred",
    "typisdefined",
    "typdelim",
    "typrelid",
    "typsubscript",
    "typelem",
    "typarray",
    "typinput",
    "typoutput",
    "typreceive",
    "typsend",
    "typmodin",
    "typmodout",
    "typanalyze",
    "typalign",
    "typstorage",
    "typnotnull",
    "typbasetype",
    "typtypmod",
    "typndims",
    "typcollation",
    "typdefaultbin",
    "typdefault",
    "typacl",
];

pub(super) fn inventory(snapshot: &Snapshot) -> Inventory<'_> {
    let mut result = BTreeMap::new();
    for record in records(snapshot, "types") {
        identity::keys(&record.value["raw"], FIELDS);
        identity::keys(
            record.value,
            &[
                "raw",
                "identity",
                "owner",
                "formatted_type",
                "element",
                "array",
                "relation",
                "base",
                "default",
                "handlers",
            ],
        );
        identity::object(&record.value["identity"]);
        assert_eq!(record.value["identity"]["schema"], "public");
        let key = identity::text(&record.value["identity"]["identity"]).to_owned();
        assert!(
            result.insert(key, record).is_none(),
            "duplicate type identity"
        );
    }
    result
}

pub(super) fn handlers(value: &Value) {
    let helpers = value["handlers"].as_array().unwrap();
    assert_eq!(helpers.len(), HANDLERS.len());
    for (field, helper) in HANDLERS.iter().zip(helpers) {
        identity::keys(helper, &["field", "oid", "identity", "rendered"]);
        assert_eq!(
            helper["field"], *field,
            "ordered complete regproc witnesses"
        );
        assert_eq!(
            value["raw"][field], helper["rendered"],
            "raw regproc rendering witness"
        );
        let id = identity::oid(&helper["oid"]);
        if id == "0" {
            assert!(helper["identity"].is_null());
            assert_eq!(helper["rendered"], "-");
        } else {
            identity::object(&helper["identity"]);
            assert!(
                id.parse::<u32>().unwrap() < 16384,
                "source has no non-built-in type handlers"
            );
            assert_eq!(helper["identity"]["schema"], "pg_catalog");
            let name = identity::text(&helper["rendered"]);
            assert!(!name.is_empty());
            assert_eq!(
                identity::text(&helper["identity"]["identity"])
                    .split_once('(')
                    .unwrap()
                    .0,
                format!("pg_catalog.{name}")
            );
            if !helper["identity"]["name"].is_null() {
                assert_eq!(helper["identity"]["name"], name);
            }
        }
    }
}

pub(super) fn validate(structure: &Structure<'_>) {
    for record in structure.types.values() {
        let value = record.value;
        let raw = &value["raw"];
        identity::owner(structure.snapshot, &raw["typowner"], &value["owner"]);
        identity::namespace(structure.snapshot, &raw["typnamespace"], "public");
        handlers(value);
        identity::class_link(structure, &raw["typrelid"], &value["relation"]);
        for (field, helper) in [
            ("typelem", "element"),
            ("typarray", "array"),
            ("typbasetype", "base"),
        ] {
            identity::type_link(structure, &raw[field], &value[helper]);
        }
        if identity::oid(&raw["typrelid"]) != "0" {
            let relation = structure.relations[&structure.classes[identity::oid(&raw["typrelid"])]];
            assert_eq!(
                relation.value["raw"]["reltype"], raw["oid"],
                "reciprocal row type link"
            );
        }
        if identity::oid(&raw["typarray"]) != "0" {
            let array = structure.types[&structure.type_ids[identity::oid(&raw["typarray"])]];
            reciprocal(raw, &array.value["raw"]);
        }
    }
}

pub(super) fn reciprocal(row: &Value, array: &Value) {
    assert_eq!(
        array["typelem"], row["oid"],
        "reciprocal array element link"
    );
    assert_eq!(row["typarray"], array["oid"], "reciprocal row array link");
}

fn new_handlers(value: &Value, array: bool) {
    let names = if array {
        [
            "array_typanalyze",
            "array_in",
            "-",
            "-",
            "array_out",
            "array_recv",
            "array_send",
            "array_subscript_handler",
        ]
    } else {
        [
            "-",
            "record_in",
            "-",
            "-",
            "record_out",
            "record_recv",
            "record_send",
            "-",
        ]
    };
    for (field, expected) in HANDLERS.iter().zip(names) {
        assert_eq!(value["raw"][field], expected);
    }
}

pub(super) fn compare(
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    fresh: &Structure<'_>,
    tables: &BTreeSet<String>,
) {
    let mut additions = BTreeSet::new();
    for table in tables {
        let relation = fresh.relations[&format!("public.{table}")];
        let key = &fresh.type_ids[identity::oid(&relation.value["raw"]["reltype"])];
        let row = fresh.types[key];
        assert_eq!(row.value["raw"]["typtype"], "c");
        assert_eq!(row.value["relation"], relation.value["identity"]);
        new_handlers(row.value, false);
        let array_key = &fresh.type_ids[identity::oid(&row.value["raw"]["typarray"])];
        let array = fresh.types[array_key];
        assert_eq!(array.value["raw"]["typtype"], "b");
        assert_eq!(identity::oid(&array.value["raw"]["typrelid"]), "0");
        assert_eq!(identity::oid(&array.value["raw"]["typarray"]), "0");
        new_handlers(array.value, true);
        reciprocal(&row.value["raw"], &array.value["raw"]);
        for added in [key, array_key] {
            assert!(!old.types.contains_key(added));
            assert!(
                additions.insert(added.clone()),
                "one row and one array per admitted table"
            );
        }
    }
    assert_eq!(additions.len(), 62);
    let expected: BTreeSet<_> = old
        .types
        .keys()
        .cloned()
        .chain(additions.iter().cloned())
        .collect();
    assert_eq!(
        fresh.types.keys().cloned().collect::<BTreeSet<_>>(),
        expected
    );
    assert_eq!(
        upgraded.types.keys().cloned().collect::<BTreeSet<_>>(),
        expected
    );
    for (key, record) in &old.types {
        exact(*record, upgraded.types[key], key);
    }
    for key in &additions {
        assert!(
            !old.type_ids
                .contains_key(identity::oid(&upgraded.types[key].value["raw"]["oid"]))
        );
    }
    for key in &expected {
        assert_eq!(
            normalized(upgraded, key),
            normalized(fresh, key),
            "complete canonical type {key}"
        );
    }
}

fn normalized(structure: &Structure<'_>, key: &str) -> Semantic {
    let record = structure.types[key];
    let value = record.value;
    let raw = &value["raw"];
    let mut result = semantic(record);
    replace(&mut result, "raw.oid", &json!(key));
    replace(
        &mut result,
        "raw.typrelid",
        &identity::class_link(structure, &raw["typrelid"], &value["relation"]),
    );
    for (field, helper) in [
        ("typelem", "element"),
        ("typarray", "array"),
        ("typbasetype", "base"),
    ] {
        replace(
            &mut result,
            &format!("raw.{field}"),
            &identity::type_link(structure, &raw[field], &value[helper]),
        );
    }
    // Raw regproc text AND all eight numeric/signature witnesses remain exact.
    result
}
