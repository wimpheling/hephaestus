use super::super::catalog::Snapshot;
use super::metadata::{self, object, oid, text};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub type Routines<'a> = BTreeMap<String, &'a Value>;

pub fn rows<'a>(snapshot: &'a Snapshot, name: &str) -> &'a [Value] {
    snapshot
        .domains()
        .get(name)
        .expect("missing catalog domain")
        .json()
        .as_array()
        .expect("complete catalog array")
}

pub fn routines(snapshot: &Snapshot) -> Routines<'_> {
    assert!(
        rows(snapshot, "aggregates").is_empty(),
        "source defines no public aggregates"
    );
    let mut routines = BTreeMap::new();
    let mut oids = BTreeSet::new();
    for routine in rows(snapshot, "routines") {
        metadata::validate(routine);
        assert!(oids.insert(oid(&routine["raw"]["oid"])));
        let key = metadata::signature(
            text(&routine["raw"]["proname"]),
            &metadata::input_types(routine),
        );
        assert!(
            routines.insert(key, routine).is_none(),
            "duplicate routine identity"
        );
    }
    routines
}

pub fn language(snapshot: &Snapshot, routine: &Value) -> Value {
    let raw = &routine["language"];
    let found: Vec<_> = rows(snapshot, "languages")
        .iter()
        .filter(|entry| entry["raw"]["oid"] == raw["oid"])
        .collect();
    assert_eq!(found.len(), 1);
    let entry = found[0];
    assert_eq!(&entry["raw"], raw);
    metadata::keys(
        entry,
        &[
            "raw",
            "owner",
            "call_handler",
            "inline_handler",
            "validator",
        ],
    );
    metadata::keys(
        raw,
        &[
            "oid",
            "lanname",
            "lanowner",
            "lanispl",
            "lanpltrusted",
            "lanplcallfoid",
            "laninline",
            "lanvalidator",
            "lanacl",
        ],
    );
    let owner: Vec<_> = rows(snapshot, "roles")
        .iter()
        .filter(|role| role["oid"] == raw["lanowner"])
        .collect();
    assert_eq!(owner.len(), 1);
    assert_eq!(entry["owner"], owner[0]["rolname"]);
    let mut normalized = object(raw).clone();
    normalized.insert("oid".into(), raw["lanname"].clone());
    normalized.insert("lanowner".into(), entry["owner"].clone());
    for (field, identity) in [
        ("lanplcallfoid", "call_handler"),
        ("laninline", "inline_handler"),
        ("lanvalidator", "validator"),
    ] {
        if raw[field] == "0" {
            assert!(entry[identity].is_null());
        } else {
            oid(&raw[field]);
            assert!(entry[identity].is_object());
            normalized.insert(field.into(), entry[identity].clone());
        }
    }
    Value::Object(normalized)
}

pub fn context(before: &Snapshot, after: &Snapshot, reference: &Snapshot) {
    assert_eq!(
        before.context(),
        after.context(),
        "target must remain the same database/context"
    );
    let target = after.context().json();
    let fresh = reference.context().json();
    assert_ne!(
        target["database"], fresh["database"],
        "reference must be an independent database"
    );
    let mut expected = target.clone();
    expected["database"] = fresh["database"].clone();
    assert_eq!(
        &expected, fresh,
        "only reference database name may differ in context"
    );
    for snapshot in [before, after, reference] {
        let databases = rows(snapshot, "database");
        assert_eq!(databases.len(), 1);
        assert_eq!(
            databases[0]["raw"]["datname"],
            snapshot.context().json()["database"]
        );
        assert_eq!(databases[0]["owner"], "postgres");
    }
}

pub fn native_members(snapshot: &Snapshot, expected: &BTreeSet<String>) -> bool {
    let extensions: Vec<_> = rows(snapshot, "extensions")
        .iter()
        .filter(|entry| entry["raw"]["extname"] == "pgcrypto")
        .collect();
    if extensions.is_empty() {
        assert!(
            !rows(snapshot, "extension_members")
                .iter()
                .any(|entry| entry["extension"]["extname"] == "pgcrypto")
        );
        return false;
    }
    assert_eq!(extensions.len(), 1);
    assert_eq!(extensions[0]["raw"]["extversion"], "1.3");
    assert_eq!(extensions[0]["owner"], "postgres");
    assert_eq!(extensions[0]["namespace"]["nspname"], "public");
    assert_eq!(
        extensions[0]["raw"]["extnamespace"],
        extensions[0]["namespace"]["oid"]
    );
    let routines = routines(snapshot);
    let by_oid: BTreeMap<_, _> = routines
        .iter()
        .map(|(key, routine)| (oid(&routine["raw"]["oid"]), (key, *routine)))
        .collect();
    let mut actual = BTreeSet::new();
    for entry in rows(snapshot, "extension_members") {
        if entry["extension"]["extname"] != "pgcrypto" {
            continue;
        }
        assert_eq!(entry["extension"], extensions[0]["raw"]);
        let raw = &entry["raw"];
        assert_eq!(raw["classid"], "1255"); // PG17 pg_proc catalog OID.
        assert_eq!(raw["objsubid"], 0);
        assert_eq!(raw["refobjid"], extensions[0]["raw"]["oid"]);
        assert_eq!(raw["deptype"], "e");
        let (key, routine) = by_oid
            .get(oid(&raw["objid"]))
            .expect("member must be a captured public routine");
        assert_eq!(entry["identity"], routine["identity"]);
        assert_eq!(entry["address"], routine["address"]);
        assert!(actual.insert((**key).clone()));
    }
    assert_eq!(
        &actual, expected,
        "native SQL declares the entire finite pgcrypto routine member set"
    );
    true
}
