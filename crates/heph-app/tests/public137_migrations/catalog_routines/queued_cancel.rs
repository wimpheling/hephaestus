use super::super::fixture;
use super::{
    Declared, Snapshot, assert_source, declarations, inventory, metadata, policy, preclosure142,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn assert_queued_cancel140(routine: &Value, entry: &Declared, checkpoint: bool) {
    if !entry.name.ends_with("140")
        && !matches!(
            entry.name.as_str(),
            "invocation119_queued_cleanup_zero_history"
                | "assert_invocation119_queued_cleanup_transition"
                | "validate_invocation119_queued_cleanup"
                | "run_cleanup_wake_original118"
                | "run_cleanup_wake_closure118"
                | "invocation119_cancel_closure"
                | "invocation119_cancel_original"
                | "invocation119_queued_cleanup_events"
                | "invocation119_cleanup_only_matches"
                | "invocation119_remove_resume_matches"
                | "guard_remove_resume_admission118"
                | "load_existing_instance_closure118"
        )
    {
        return;
    }
    if !checkpoint && super::terminal143::contains(&entry.name) {
        return;
    }
    let source = fixture::MIGRATIONS
        .iter()
        .find(|migration| {
            migration.version
                == if !checkpoint && preclosure142::replaces140(&entry.name) {
                    142
                } else {
                    140
                }
        })
        .unwrap();
    let declaration = format!("FUNCTION {}(", entry.name);
    let (_, tail) = source
        .sql
        .split_once(&declaration)
        .expect("Source140 exact declaration");
    let (_, tail) = tail.split_once(" AS $$").unwrap();
    let body = tail.split_once("$$;").unwrap().0;
    assert_eq!(routine["raw"]["prosrc"], body);
    assert_eq!(routine["owner"], "postgres");
    assert_eq!(routine["raw"]["prosecdef"], true);
    assert_eq!(routine["raw"]["prokind"], "f");
    assert_eq!(
        routine["language"]["lanname"],
        if entry.name == "invocation119_remove_resume_matches" {
            "sql"
        } else {
            "plpgsql"
        }
    );
    assert_eq!(
        routine["raw"]["proconfig"],
        json!(["search_path=pg_catalog, public, pg_temp"])
    );
    let grant = format!("GRANT EXECUTE ON FUNCTION {}(", entry.name);
    let acl = if entry.name == "load_existing_instance_closure118" {
        json!(["postgres=X/postgres", "hephaestus_app=X/postgres"])
    } else if source.sql.contains(&grant) {
        json!(["postgres=X/postgres", "hephaestus_worker=X/postgres"])
    } else {
        json!(["postgres=X/postgres"])
    };
    assert_eq!(
        routine["raw"]["proacl"], acl,
        "Source140 exact owner and declared executor ACL"
    );
}

pub(super) fn assert_forward140(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    let old = inventory::routines(before);
    let current = inventory::routines(after);
    let policy = policy();
    let declarations = declarations(&policy);
    let added: BTreeSet<_> = declarations
        .iter()
        .filter(|(_, entry)| entry.name.ends_with("140"))
        .map(|(key, _)| key.clone())
        .collect();
    assert_eq!(added.len(), 30);
    assert!(added.iter().all(|key| !old.contains_key(key)));
    assert_eq!(
        current.keys().cloned().collect::<BTreeSet<_>>(),
        old.keys().cloned().chain(added.iter().cloned()).collect()
    );
    let mut replaced = 0;
    for (key, original) in &old {
        let following = current[key];
        let mut expected = (**original).clone();
        if matches!(
            key.as_str(),
            "public.invocation119_queued_cleanup_zero_history(public.runs)"
                | "public.assert_invocation119_queued_cleanup_transition(pg_catalog.uuid,pg_catalog.text)"
                | "public.validate_invocation119_queued_cleanup(public.runs,public.runs)"
                | "public.run_cleanup_wake_original118(pg_catalog.uuid)"
                | "public.run_cleanup_wake_closure118(pg_catalog.uuid,pg_catalog.uuid,pg_catalog.jsonb)"
                | "public.invocation119_cancel_closure(pg_catalog.uuid,pg_catalog.uuid,pg_catalog.jsonb,pg_catalog.jsonb)"
                | "public.invocation119_cancel_original(pg_catalog.uuid,pg_catalog.jsonb)"
                | "public.invocation119_queued_cleanup_events(public.runs,pg_catalog.bool)"
                | "public.invocation119_cleanup_only_matches(pg_catalog.uuid,pg_catalog.uuid,pg_catalog.jsonb,pg_catalog.text,pg_catalog.text,pg_catalog.bytea,pg_catalog.bool)"
                | "public.invocation119_remove_resume_matches(pg_catalog.uuid,pg_catalog.text,pg_catalog.text,pg_catalog.jsonb)"
                | "public.guard_remove_resume_admission118()"
                | "public.load_existing_instance_closure118(pg_catalog.uuid,pg_catalog.uuid,pg_catalog.uuid,pg_catalog.bytea,pg_catalog.uuid,pg_catalog.jsonb,pg_catalog.jsonb,pg_catalog.text,pg_catalog.text,pg_catalog.jsonb)"
        ) {
            replaced += 1;
            assert_checkpoint140(following, declarations[key]);
            expected["raw"]["prosrc"] = following["raw"]["prosrc"].clone();
            expected["definition"] = following["definition"].clone();
        }
        assert_eq!(
            expected, *following,
            "complete139 routine identity/metadata {key}"
        );
    }
    assert_eq!(replaced, 12, "twelve exact139 signatures replaced by140");
    let old_oids: BTreeSet<_> = old
        .values()
        .map(|routine| metadata::oid(&routine["raw"]["oid"]))
        .collect();
    added
        .iter()
        .map(|key| {
            let routine = current[key];
            assert_checkpoint140(routine, declarations[key]);
            let function_oid = metadata::oid(&routine["raw"]["oid"]);
            assert!(!old_oids.contains(function_oid));
            function_oid.to_owned()
        })
        .collect()
}

fn assert_checkpoint140(routine: &Value, entry: &Declared) {
    if preclosure142::replaces140(&entry.name) {
        assert_queued_cancel140(routine, entry, true);
    } else {
        assert_source(routine, entry);
    }
}
