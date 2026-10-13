use super::super::catalog::Snapshot;
use super::{Inventory, Semantic, Structure, exact, identity, records, replace, rewrite, semantic};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const FIELDS: &[&str] = &[
    "oid",
    "relname",
    "relnamespace",
    "reltype",
    "reloftype",
    "relowner",
    "relam",
    "reltablespace",
    "reltoastrelid",
    "relhasindex",
    "relisshared",
    "relpersistence",
    "relkind",
    "relnatts",
    "relchecks",
    "relhasrules",
    "relhastriggers",
    "relhassubclass",
    "relrowsecurity",
    "relforcerowsecurity",
    "relispopulated",
    "relreplident",
    "relispartition",
    "relrewrite",
    "relacl",
    "reloptions",
    "relpartbound",
];

pub(super) fn inventory(snapshot: &Snapshot) -> Inventory<'_> {
    let records = records(snapshot, "relations");
    let mut result = BTreeMap::new();
    for record in &records {
        let value = record.value;
        identity::keys(&value["raw"], FIELDS);
        identity::keys(
            value,
            &[
                "raw",
                "identity",
                "namespace",
                "owner",
                "access_method",
                "tablespace",
                "row_type",
                "toast_parent",
                "toast_index_parent",
                "partition_bound",
            ],
        );
        identity::object(&value["identity"]);
        if value["namespace"] == "public" {
            assert_eq!(value["identity"]["schema"], "public");
            assert!(value["toast_parent"].is_null() && value["toast_index_parent"].is_null());
            let key = format!("public.{}", identity::text(&value["raw"]["relname"]));
            assert!(
                result.insert(key, *record).is_none(),
                "duplicate public relation"
            );
        }
    }
    let public = result.clone();
    for record in records {
        let value = record.value;
        if value["namespace"] == "public" {
            continue;
        }
        assert_eq!(value["namespace"], "pg_toast");
        let index = value["raw"]["relkind"] == "i";
        assert_eq!(value["raw"]["relkind"], if index { "i" } else { "t" });
        let helper = if index {
            "toast_index_parent"
        } else {
            "toast_parent"
        };
        assert!(
            value[if index {
                "toast_parent"
            } else {
                "toast_index_parent"
            }]
            .is_null()
        );
        let parents: Vec<_> = public
            .iter()
            .filter(|(_, parent)| parent.value["identity"] == value[helper])
            .collect();
        assert_eq!(parents.len(), 1, "unique generated relation parent");
        let (parent_key, parent) = parents[0];
        assert_eq!(parent.value["raw"]["relkind"], "r");
        let name = identity::generated_name(identity::oid(&parent.value["raw"]["oid"]), index);
        assert_eq!(value["raw"]["relname"], name, "finite generated name");
        let key = format!(
            "{}({parent_key})",
            if index { "toast_index" } else { "toast" }
        );
        identity::toast_identity(&value["identity"], &name, &key);
        assert!(
            result.insert(key, record).is_none(),
            "duplicate generated relation"
        );
    }
    result
}

pub(super) fn validate(structure: &Structure<'_>) {
    for (key, record) in &structure.relations {
        let value = record.value;
        let raw = &value["raw"];
        identity::owner(structure.snapshot, &raw["relowner"], &value["owner"]);
        identity::namespace(
            structure.snapshot,
            &raw["relnamespace"],
            identity::text(&value["namespace"]),
        );
        identity::type_link(structure, &raw["reltype"], &value["row_type"]);
        for field in ["reloftype", "relrewrite"] {
            assert_eq!(identity::oid(&raw[field]), "0");
        }
        assert_eq!(raw["relispartition"], false);
        assert!(raw["relpartbound"].is_null() && value["partition_bound"].is_null());
        match identity::oid(&raw["relam"]) {
            "0" => assert!(value["access_method"].is_null()),
            "2" => assert_eq!(value["access_method"], "heap"),
            "403" => assert_eq!(value["access_method"], "btree"),
            other => panic!("unqualified access method {other}"),
        }
        if identity::oid(&raw["reltablespace"]) == "0" {
            assert!(value["tablespace"].is_null());
        } else {
            let database = identity::rows(structure.snapshot, "database");
            assert_eq!(database.len(), 1);
            assert_eq!(raw["reltablespace"], database[0]["tablespace"]["oid"]);
            assert_eq!(value["tablespace"], database[0]["tablespace"]["spcname"]);
        }
        let toast = identity::oid(&raw["reltoastrelid"]);
        if toast != "0" {
            assert_eq!(raw["relkind"], "r");
            assert_eq!(structure.classes[toast], format!("toast({key})"));
            assert!(
                structure
                    .relations
                    .contains_key(&format!("toast_index({key})"))
            );
        } else if key.starts_with("public.") {
            assert!(!structure.relations.contains_key(&format!("toast({key})")));
        }
    }
}

pub(super) fn admitted_additions(
    old: &Structure<'_>,
    fresh: &Structure<'_>,
    tables: &BTreeSet<String>,
    indexes: BTreeSet<String>,
) -> BTreeSet<String> {
    assert_eq!(indexes.len(), 96);
    let mut result = indexes;
    assert!(result.insert("public.one_active_run_volume_slot".to_owned()));
    for table in tables {
        let key = format!("public.{table}");
        assert!(!old.relations.contains_key(&key));
        let canonical = fresh.relations[&key].value;
        assert_eq!(canonical["raw"]["relkind"], "r");
        assert!(result.insert(key.clone()));
        if identity::oid(&canonical["raw"]["reltoastrelid"]) != "0" {
            assert!(result.insert(format!("toast({key})")));
            assert!(result.insert(format!("toast_index({key})")));
        }
    }
    for key in &result {
        assert!(
            !old.relations.contains_key(key),
            "source additions cannot rebuild old relations"
        );
        if key.starts_with("public.") && !tables.contains(key.strip_prefix("public.").unwrap()) {
            assert_eq!(fresh.relations[key].value["raw"]["relkind"], "i");
        }
    }
    result
}

fn checks(snapshot: &Snapshot, relation: &Value) -> usize {
    identity::rows(snapshot, "constraints")
        .iter()
        .filter(|constraint| {
            constraint["raw"]["conrelid"] == relation["raw"]["oid"]
                && constraint["raw"]["contype"] == "c"
        })
        .count()
}

pub(super) fn preserve(
    key: &str,
    old: &Structure<'_>,
    upgraded: &Structure<'_>,
    additions: &BTreeMap<String, usize>,
    replaced_children: &BTreeSet<String>,
) {
    if replaced_children.contains(key) {
        // Qualification preserves every captured child field except typed physical links.
        assert_eq!(normalized(old, key), normalized(upgraded, key));
        return;
    }
    let original = old.relations[key];
    let current = upgraded.relations[key];
    if key == "public.secret_runtime_mounts" {
        preserve_secret_mounts(original, current);
        return;
    }
    let table = key.strip_prefix("public.").unwrap_or("");
    let mut expected = semantic(original);
    let mut actual = semantic(current);
    if key == rewrite::PARENT {
        assert_eq!(replaced_children, &rewrite::children());
        rewrite::parent_link(old, &mut expected);
        rewrite::parent_link(upgraded, &mut actual);
    }
    if let Some(count) = additions.get(table) {
        let old_count = original.value["raw"]["relnatts"].as_u64().unwrap();
        replace(
            &mut expected,
            "raw.relnatts",
            &json!(old_count + u64::try_from(*count).unwrap()),
        );
        let previous = checks(old.snapshot, original.value);
        let following = checks(upgraded.snapshot, current.value);
        assert_eq!(original.value["raw"]["relchecks"], previous);
        assert_eq!(current.value["raw"]["relchecks"], following);
        let pair = match table {
            "runs" => (7, 11),
            "agent_instance_volume_leases" => (4, 6),
            "agent_instance_state_volumes" => (8, 9),
            "recipe_execution_transitions" => (previous, previous),
            "agent_instance_removal_requests" => (4, 5),
            _ => panic!("undeclared changed relation"),
        };
        assert_eq!((previous, following), pair);
        replace(&mut expected, "raw.relchecks", &json!(following));
    }
    if key == "public.command_inbox" {
        assert_eq!(original.value["raw"]["relhastriggers"], false);
        assert_eq!(current.value["raw"]["relhastriggers"], true);
        replace(&mut expected, "raw.relhastriggers", &json!(true));
    }
    if additions.contains_key(table) || key == "public.command_inbox" {
        assert_eq!(expected, actual, "complete old relation preservation {key}");
    } else {
        exact(original, current, key);
    }
}

// Source141 changes only this table's ACL. Retain every other lossless field.
pub(super) fn preserve_secret_mounts(original: super::Record<'_>, current: super::Record<'_>) {
    assert_eq!(original.value["owner"], "postgres");
    assert_eq!(original.value["raw"]["relkind"], "r");
    assert!(original.value["raw"]["relacl"].is_null());
    let acl = json!([
        "postgres=arwdDxtm/postgres",
        "hephaestus_worker=arw/postgres"
    ]);
    assert_eq!(
        current.value["raw"]["relacl"], acl,
        "Source141 exact worker SIU"
    );
    let mut expected = semantic(original);
    let mut actual = semantic(current);
    assert!(
        serde_json::from_str::<Value>(&expected["raw.relacl"])
            .unwrap()
            .is_null(),
        "original captured ACL text must retain NULL"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&actual["raw.relacl"]).unwrap(),
        acl,
        "captured ACL text must match Source141 exact worker SIU"
    );
    replace(&mut expected, "raw.relacl", &acl);
    // Both sides use the exact typed ACL above; native JSONB spacing is not a grant.
    replace(&mut actual, "raw.relacl", &acl);
    assert_eq!(
        expected, actual,
        "complete old relation preservation public.secret_runtime_mounts"
    );
}

pub(super) fn normalized(structure: &Structure<'_>, key: &str) -> Semantic {
    let record = structure.relations[key];
    let value = record.value;
    let raw = &value["raw"];
    let mut result = semantic(record);
    replace(&mut result, "raw.oid", &json!(key));
    replace(
        &mut result,
        "raw.reltype",
        &identity::type_link(structure, &raw["reltype"], &value["row_type"]),
    );
    let toast = identity::oid(&raw["reltoastrelid"]);
    replace(
        &mut result,
        "raw.reltoastrelid",
        &json!(if toast == "0" {
            "0"
        } else {
            &structure.classes[toast]
        }),
    );
    if value["namespace"] == "pg_toast" {
        replace(&mut result, "raw.relname", &json!(key));
        replace(
            &mut result,
            "identity",
            &identity::toast_identity(&value["identity"], identity::text(&raw["relname"]), key),
        );
    }
    result
}
