use super::{Inventory, Record, Semantic, Structure, identity, json, records, replace, semantic};

pub(super) const FIELDS: &[&str] = &[
    "oid",
    "extname",
    "extowner",
    "extnamespace",
    "extrelocatable",
    "extversion",
    "extconfig",
    "extcondition",
];

pub(super) fn inventory<'a>(structure: &Structure<'a>) -> Inventory<'a> {
    let mut result = Inventory::new();
    let mut ids = std::collections::BTreeSet::new();
    for record in records(structure.snapshot, "extensions") {
        let entry = record.value;
        identity::keys(entry, &["raw", "namespace", "owner", "config_relations"]);
        identity::keys(&entry["raw"], FIELDS);
        let raw = &entry["raw"];
        assert_ne!(identity::oid(&raw["oid"]), "0");
        assert!(ids.insert(identity::oid(&raw["oid"])));
        identity::owner(structure.snapshot, &raw["extowner"], &entry["owner"]);
        let namespace = &entry["namespace"];
        identity::keys(namespace, &["oid", "nspname", "nspowner", "nspacl"]);
        assert_eq!(raw["extnamespace"], namespace["oid"]);
        if namespace["nspname"] == "public" {
            let witness = identity::rows(structure.snapshot, "namespaces");
            assert_eq!(witness.len(), 1);
            assert_eq!(witness[0]["raw"], *namespace);
        } else {
            assert_eq!(namespace["nspname"], "pg_catalog");
            assert_eq!(namespace["oid"], "11");
        }
        if namespace["nspname"] == "pg_catalog" {
            identity::owner(
                structure.snapshot,
                &namespace["nspowner"],
                &json!("postgres"),
            );
        }
        // public may be owned by pg_database_owner on PG17; its captured owner
        // is independently joined rather than substituted with postgres.
        if namespace["nspname"] == "public" {
            let witness = &identity::rows(structure.snapshot, "namespaces")[0];
            identity::owner(
                structure.snapshot,
                &namespace["nspowner"],
                &witness["owner"],
            );
        }
        let name = identity::text(&raw["extname"]);
        if name == "pgcrypto" {
            native(entry);
        } else {
            assert_eq!(name, "plpgsql", "normal source-qualified extension profile");
        }
        assert!(result.insert(name.to_owned(), record).is_none());
    }
    result
}

pub(super) fn native(entry: &serde_json::Value) {
    let raw = &entry["raw"];
    assert_eq!(raw["extname"], "pgcrypto");
    assert_eq!(entry["owner"], "postgres");
    assert_eq!(entry["namespace"]["nspname"], "public");
    assert_eq!(raw["extversion"], "1.3");
    assert_eq!(raw["extrelocatable"], true);
    for field in ["extconfig", "extcondition"] {
        assert!(
            raw[field].is_null(),
            "native pgcrypto has no config/condition"
        );
    }
    assert!(entry["config_relations"].is_null());
}

pub(super) fn normalized(record: Record<'_>) -> Semantic {
    let mut result = semantic(record);
    if identity::oid(&record.value["raw"]["oid"])
        .parse::<u32>()
        .unwrap()
        >= 16384
    {
        replace(&mut result, "raw.oid", &record.value["raw"]["extname"]);
    }
    result
}
