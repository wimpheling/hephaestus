use super::{
    Inventory, Record, Semantic, Structure, builtin, compare_domain, embedded_extension, extension,
    identity, json, records, replace, routine, semantic, signed,
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
    // The preceding complete routine oracle has already independently proved
    // that these fresh members are exactly the reviewed native 36 signatures.
    let native: BTreeSet<_> = reference
        .iter()
        .filter(|(_, record)| record.value["extension"]["extname"] == "pgcrypto")
        .map(|(key, _)| key.clone())
        .collect();
    assert_eq!(native.len(), 36);
    let expected = original.keys().cloned().chain(native).collect();
    compare_domain(
        &original, &current, &reference, &expected, normalized, normalized,
    );
}

fn inventory<'a>(structure: &Structure<'a>, extensions: &Inventory<'_>) -> Inventory<'a> {
    let mut result = Inventory::new();
    let mut addresses = BTreeSet::new();
    for record in records(structure.snapshot, "extension_members") {
        let entry = record.value;
        identity::keys(entry, &["raw", "extension", "identity", "address"]);
        let raw = &entry["raw"];
        let name = extension(extensions, &raw["refobjid"]);
        shape(raw);
        assert_eq!(entry["extension"], extensions[name].value["raw"]);
        identity::object(&entry["identity"]);
        identity::keys(&entry["address"], &["type", "object_names", "object_args"]);
        if name == "pgcrypto" {
            assert_eq!(raw["classid"], "1255");
            let function = routine(structure, &raw["objid"], &entry["identity"]);
            assert_eq!(entry["address"], function["address"]);
            assert_eq!(function["language"]["lanname"], "c");
            assert_eq!(function["raw"]["probin"], "$libdir/pgcrypto");
        } else {
            assert_eq!(name, "plpgsql");
            plpgsql_member(entry, plpgsql_language(structure));
        }
        let address = (
            identity::oid(&raw["classid"]),
            identity::oid(&raw["objid"]),
            signed(&raw["objsubid"]),
        );
        assert!(
            addresses.insert(address),
            "duplicate extension member address"
        );
        let key = format!("{name}/{}", entry["identity"]);
        assert!(
            result.insert(key, record).is_none(),
            "ambiguous member identity"
        );
    }
    let inherited: BTreeSet<_> = result
        .values()
        .filter(|record| record.value["extension"]["extname"] == "plpgsql")
        .map(|record| {
            (
                identity::oid(&record.value["raw"]["classid"]),
                identity::oid(&record.value["raw"]["objid"]),
            )
        })
        .collect();
    let language = plpgsql_language(structure);
    let expected = [
        ("2612", "oid"),
        ("1255", "lanplcallfoid"),
        ("1255", "laninline"),
        ("1255", "lanvalidator"),
    ]
    .into_iter()
    .map(|(class, field)| (class, identity::oid(&language["raw"][field])))
    .collect();
    assert_eq!(
        inherited, expected,
        "exact language and three inherited handlers"
    );
    result
}

fn plpgsql_language<'a>(structure: &Structure<'a>) -> &'a serde_json::Value {
    let found: Vec<_> = identity::rows(structure.snapshot, "languages")
        .iter()
        .filter(|language| language["raw"]["lanname"] == "plpgsql")
        .collect();
    assert_eq!(found.len(), 1);
    let language = found[0];
    assert_eq!(language["raw"]["lanispl"], true);
    assert_eq!(language["raw"]["lanpltrusted"], true);
    language
}

// PostgreSQL REL_17_10 plpgsql--1.0.sql declares exactly these three C handlers
// and the trusted language. Their current immutable IDs are captured via the
// complete captured pg_language row. The routine oracle covers public routines;
// pg_catalog handler bodies require the separately qualified PG17.10 assets.
pub(super) fn plpgsql_member(entry: &serde_json::Value, language: &serde_json::Value) {
    let raw = &entry["raw"];
    builtin(&raw["objid"]);
    if raw["classid"] == "2612" {
        // PG17 pg_language catalog OID.
        assert_eq!(raw["objid"], language["raw"]["oid"]);
        assert_eq!(
            entry["identity"],
            json!({"type":"language", "schema":null, "name":"plpgsql", "identity":"plpgsql"})
        );
        assert_eq!(
            entry["address"],
            json!({"type":"language", "object_names":["plpgsql"], "object_args":[]})
        );
    } else {
        assert_eq!(raw["classid"], "1255");
        let handlers = [
            (
                "lanplcallfoid",
                "call_handler",
                "plpgsql_call_handler",
                Vec::<&str>::new(),
            ),
            (
                "laninline",
                "inline_handler",
                "plpgsql_inline_handler",
                vec!["pg_catalog.internal"],
            ),
            (
                "lanvalidator",
                "validator",
                "plpgsql_validator",
                vec!["pg_catalog.oid"],
            ),
        ];
        let found: Vec<_> = handlers
            .iter()
            .filter(|(field, _, _, _)| language["raw"][*field] == raw["objid"])
            .collect();
        assert_eq!(found.len(), 1, "one of the three actual language handlers");
        let (_, helper, name, arguments) = found[0];
        assert_eq!(entry["identity"], language[*helper]);
        assert_eq!(entry["identity"]["type"], "function");
        assert_eq!(entry["identity"]["schema"], "pg_catalog");
        assert_eq!(
            entry["address"],
            json!({"type":"function", "object_names":["pg_catalog", name], "object_args":arguments})
        );
    }
}

pub(super) fn shape(raw: &serde_json::Value) {
    identity::keys(
        raw,
        &[
            "classid",
            "objid",
            "objsubid",
            "refclassid",
            "refobjid",
            "refobjsubid",
            "deptype",
        ],
    );
    assert!(matches!(identity::oid(&raw["classid"]), "1255" | "2612"));
    assert_ne!(identity::oid(&raw["objid"]), "0");
    assert_eq!(signed(&raw["objsubid"]), 0);
    assert_eq!(identity::oid(&raw["refclassid"]), "3079");
    assert_ne!(identity::oid(&raw["refobjid"]), "0");
    assert_eq!(signed(&raw["refobjsubid"]), 0);
    assert_eq!(raw["deptype"], "e");
}

pub(super) fn normalized(record: Record<'_>) -> Semantic {
    let mut result = semantic(record);
    let raw = &record.value["raw"];
    if record.value["extension"]["extname"] == "pgcrypto" {
        replace(&mut result, "raw.objid", &record.value["identity"]);
    }
    if identity::oid(&raw["refobjid"]).parse::<u32>().unwrap() >= 16384 {
        replace(
            &mut result,
            "raw.refobjid",
            &record.value["extension"]["extname"],
        );
    }
    embedded_extension(&mut result, record);
    // Catalog class OIDs and signed subobject IDs stay literal; in particular
    // pg_language2612 is not projected onto pg_proc1255.
    result
}
