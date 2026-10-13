//! Synthetic routine witnesses exercise owner preservation without a database.
use super::{normalized, validate};
use serde_json::{Value, json};

fn routine() -> Value {
    // Migration0015 deliberately assigns this trigger function to this role.
    json!({
        "raw": {
            "oid": "10001", "proname": "assign_initial_project_maintainer",
            "pronamespace": "2200", "proowner": "10002", "prolang": "13563",
            "procost": 100, "prorows": 0, "provariadic": "0", "prosupport": "-",
            "prokind": "f", "prosecdef": true, "proleakproof": false,
            "proisstrict": false, "proretset": false, "provolatile": "v",
            "proparallel": "u", "pronargs": 0, "pronargdefaults": 0,
            "prorettype": "2279", "proargtypes": [], "proallargtypes": null,
            "proargmodes": null, "proargnames": null, "proargdefaults": null,
            "protrftypes": null, "prosrc": "BEGIN RETURN NEW; END",
            "probin": null, "prosqlbody": null,
            "proconfig": ["search_path=pg_catalog, public"],
            "proacl": ["hephaestus_authz_owner=X/hephaestus_authz_owner"]
        },
        "identity": {
            "type": "function", "schema": "public",
            "name": "assign_initial_project_maintainer",
            "identity": "public.assign_initial_project_maintainer()"
        },
        "address": {
            "type": "function", "object_names": ["public", "assign_initial_project_maintainer"],
            "object_args": []
        },
        "owner": "hephaestus_authz_owner",
        "language": {"oid": "13563", "lanname": "plpgsql"},
        "kind": "f", "support_oid": "0", "arguments": "",
        "identity_arguments": "", "result": "trigger", "argument_types": null,
        "return_type": {
            "type": "type", "schema": "pg_catalog", "name": "trigger",
            "identity": "pg_catalog.trigger"
        },
        "definition": "CREATE FUNCTION public.assign_initial_project_maintainer() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RETURN NEW; END$$",
        "aggregate": null
    })
}

#[test]
fn inherited_role_owner_passes_shape_validation() {
    validate(&routine());
}

#[test]
#[should_panic(expected = "unapproved change to original routine")]
fn inherited_routine_rejects_owner_name_drift() {
    let old = routine();
    let mut current = old.clone();
    current["owner"] = json!("postgres");
    super::super::assert_old(
        "public.assign_initial_project_maintainer()",
        &old,
        &current,
        None,
    );
}

#[test]
#[should_panic(expected = "unapproved change to original routine")]
fn inherited_routine_rejects_owner_oid_drift() {
    let old = routine();
    let mut current = old.clone();
    current["raw"]["proowner"] = json!("10003");
    super::super::assert_old(
        "public.assign_initial_project_maintainer()",
        &old,
        &current,
        None,
    );
}

#[test]
fn fresh_comparison_retains_owner_identity_and_acl() {
    let target = routine();
    let language = json!({"lanname": "plpgsql", "lanowner": "postgres"});
    let mut fresh = target.clone();
    fresh["raw"]["proowner"] = json!("20002");
    assert_eq!(
        normalized(&target, &language),
        normalized(&fresh, &language)
    );

    // A legitimate cross-database role OID difference cannot hide owner drift.
    fresh["owner"] = json!("postgres");
    assert_ne!(
        normalized(&target, &language),
        normalized(&fresh, &language)
    );
    fresh["owner"] = target["owner"].clone();
    fresh["raw"]["proacl"] = json!(["=X/hephaestus_authz_owner"]);
    assert_ne!(
        normalized(&target, &language),
        normalized(&fresh, &language)
    );
}
