//! Source142 has nine additions and eight body-only replacements.
use super::{Declared, Snapshot, inventory, metadata};
use serde_json::{Value, json};
use std::collections::BTreeSet;

type Entry = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    bool,
    bool,
);
const ENTRIES: [Entry; 17] = [
    (
        "verify_invocation142_empty_secret_cleanup",
        &[
            "pg_catalog.uuid",
            "pg_catalog.bytea",
            "pg_catalog.text",
            "pg_catalog.text",
        ],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "prepared_closure_row142",
        &["public.agent_instance_removal_requests"],
        "sql",
        "s",
        true,
        false,
    ),
    (
        "prepared_drain_progress142",
        &["pg_catalog.uuid", "pg_catalog.jsonb"],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "pending_instance_drain142",
        &["pg_catalog.uuid", "pg_catalog.text"],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "prepared_closure_origin142",
        &["pg_catalog.uuid"],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "prepared_resume_inventory142",
        &[
            "pg_catalog.uuid",
            "pg_catalog.text",
            "pg_catalog.text",
            "pg_catalog.jsonb",
        ],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "prepare_resumed_prepared_instance_closure142",
        &[
            "pg_catalog.uuid",
            "pg_catalog.uuid",
            "pg_catalog.jsonb",
            "pg_catalog.jsonb",
            "pg_catalog.bytea",
            "pg_catalog.jsonb",
            "pg_catalog.jsonb",
            "pg_catalog.text",
            "pg_catalog.text",
            "pg_catalog.jsonb",
            "pg_catalog.int8",
            "pg_catalog.bytea",
            "pg_catalog.uuid",
            "pg_catalog.int8",
            "pg_catalog.bytea",
        ],
        "plpgsql",
        "v",
        true,
        true,
    ),
    (
        "guard_prepared_resume_origin142",
        &[],
        "plpgsql",
        "v",
        true,
        false,
    ),
    (
        "validate_prepared_resume_origin142",
        &[],
        "plpgsql",
        "v",
        true,
        false,
    ),
    (
        "remove_resume_context118",
        &["pg_catalog.uuid"],
        "plpgsql",
        "s",
        true,
        true,
    ),
    (
        "queued_processed_closure140",
        &["pg_catalog.uuid", "pg_catalog.uuid", "pg_catalog.jsonb"],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "queued_cancel_closure140",
        &[
            "pg_catalog.uuid",
            "pg_catalog.uuid",
            "pg_catalog.jsonb",
            "pg_catalog.jsonb",
        ],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "queued_removal_origin140",
        &["pg_catalog.uuid"],
        "plpgsql",
        "s",
        true,
        false,
    ),
    (
        "guard_remove_resume_admission118",
        &[],
        "plpgsql",
        "v",
        true,
        false,
    ),
    (
        "load_existing_instance_closure118",
        &[
            "pg_catalog.uuid",
            "pg_catalog.uuid",
            "pg_catalog.uuid",
            "pg_catalog.bytea",
            "pg_catalog.uuid",
            "pg_catalog.jsonb",
            "pg_catalog.jsonb",
            "pg_catalog.text",
            "pg_catalog.text",
            "pg_catalog.jsonb",
        ],
        "plpgsql",
        "s",
        true,
        true,
    ),
    ("recipe_command_guard", &[], "plpgsql", "v", false, false),
    (
        "validate_ordinary_queued_cancel_commit140",
        &[],
        "plpgsql",
        "v",
        true,
        false,
    ),
];

pub(super) fn replaces140(name: &str) -> bool {
    matches!(
        name,
        "validate_ordinary_queued_cancel_commit140"
            | "queued_processed_closure140"
            | "queued_cancel_closure140"
            | "queued_removal_origin140"
            | "guard_remove_resume_admission118"
            | "load_existing_instance_closure118"
    )
}

pub(super) fn assert_source(routine: &Value, declaration: &Declared) {
    if super::terminal143::contains(&declaration.name) {
        return;
    }
    assert_checkpoint(routine, declaration);
}

fn assert_checkpoint(routine: &Value, declaration: &Declared) {
    let Some(entry) = ENTRIES.iter().find(|entry| entry.0 == declaration.name) else {
        return;
    };
    let (name, types, language, volatility, security, app) = entry;
    assert_eq!(
        declaration
            .input_types
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        *types
    );
    let source = super::super::fixture::MIGRATIONS
        .iter()
        .find(|entry| entry.version == 142)
        .unwrap();
    let (_, tail) = source.sql.split_once(&format!("FUNCTION {name}(")).unwrap();
    let (_, tail) = tail.split_once(" AS $$").unwrap();
    assert_eq!(routine["raw"]["prosrc"], tail.split_once("$$;").unwrap().0);
    assert_eq!(routine["owner"], "postgres");
    assert_eq!(routine["raw"]["prokind"], "f");
    assert_eq!(routine["language"]["lanname"], *language);
    assert_eq!(routine["raw"]["provolatile"], *volatility);
    assert_eq!(routine["raw"]["prosecdef"], *security);
    assert_eq!(routine["raw"]["proparallel"], "u");
    assert_eq!(routine["raw"]["pronargdefaults"], 0);
    assert_eq!(
        routine["raw"]["proconfig"],
        json!([if *name == "recipe_command_guard" {
            "search_path=pg_catalog, public"
        } else {
            "search_path=pg_catalog, public, pg_temp"
        }])
    );
    assert_eq!(
        routine["raw"]["proacl"],
        if *name == "verify_invocation142_empty_secret_cleanup" {
            json!(["postgres=X/postgres", "hephaestus_worker=X/postgres"])
        } else if *app {
            json!(["postgres=X/postgres", "hephaestus_app=X/postgres"])
        } else {
            json!(["postgres=X/postgres"])
        }
    );
}

pub(super) fn forward(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    let old = inventory::routines(before);
    let current = inventory::routines(after);
    let policy = super::policy();
    let declarations = super::declarations(&policy);
    let added: BTreeSet<_> = ENTRIES
        .iter()
        .filter(|entry| {
            entry.0.ends_with("142") || entry.0 == "verify_invocation142_empty_secret_cleanup"
        })
        .map(|entry| {
            metadata::signature(
                entry.0,
                &entry.1.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(added.len(), 9);
    assert_eq!(
        current.keys().cloned().collect::<BTreeSet<_>>(),
        old.keys().cloned().chain(added.iter().cloned()).collect()
    );
    for (key, original) in &old {
        let following = current[key];
        let mut expected = (**original).clone();
        if ENTRIES
            .iter()
            .any(|entry| entry.0 == metadata::text(&original["raw"]["proname"]))
        {
            assert_checkpoint(following, declarations[key]);
            expected["raw"]["prosrc"] = following["raw"]["prosrc"].clone();
            expected["definition"] = following["definition"].clone();
        }
        assert_eq!(
            expected, *following,
            "only exact142 bodies replace old routines"
        );
    }
    added
        .iter()
        .map(|key| {
            let routine = current[key];
            assert_checkpoint(routine, declarations[key]);
            let id = metadata::oid(&routine["raw"]["oid"]);
            assert!(old.values().all(|v| metadata::oid(&v["raw"]["oid"]) != id));
            id.to_owned()
        })
        .collect()
}
