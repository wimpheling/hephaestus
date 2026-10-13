//! Exact Source140 declarations and catalog hooks; no helper is executed.
use super::super::{Postgres, Transaction, invalid};
use sqlx::FromRow;
#[path = "preclosure_drain.rs"]
mod preclosure_drain;
#[path = "terminal143.rs"]
mod terminal143;

const SOURCE: &str =
    include_str!("../../../../../../../migrations/0140_ordinary_queued_invocation_cancel.sql");

struct Entry {
    name: &'static str,
    signature: &'static str,
    arguments: &'static str,
    result: &'static str,
    names: &'static [&'static str],
    types: &'static [&'static str],
    modes: &'static [&'static str],
    table_result: bool,
    returns_set: bool,
    volatility: &'static str,
    language: &'static str,
    worker: bool,
}

#[path = "queued_cancel/entries.rs"]
mod entries;
use entries::ENTRIES;

// These flags check independent native metadata, rather than a Rust state model.
#[allow(clippy::struct_excessive_bools)]
#[derive(FromRow)]
struct Routine {
    name: String,
    arguments: String,
    result: String,
    names: Option<Vec<String>>,
    types: Option<Vec<String>>,
    modes: Option<Vec<String>>,
    table_result: bool,
    returns_set: bool,
    volatility: String,
    body: String,
    shape: bool,
    exact_acl: bool,
}

pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let names: Vec<_> = ENTRIES.iter().map(|entry| entry.name).collect();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_proc AS routine
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid=routine.pronamespace
         WHERE namespace.nspname='public' AND routine.proname::text=ANY($1::text[])",
    )
    .bind(&names)
    .fetch_one(&mut **admin)
    .await?;
    if count != 42 {
        return Err(invalid("Source140 routine inventory differs"));
    }
    for entry in &ENTRIES {
        verify_routine(admin, entry).await?;
    }
    verify_triggers(admin).await?;
    preclosure_drain::verify(admin).await?;
    terminal143::verify(admin).await
}

async fn verify_routine(
    admin: &mut Transaction<'_, Postgres>,
    entry: &Entry,
) -> Result<(), sqlx::Error> {
    let observed: Option<Routine> = sqlx::query_as(
        "SELECT routine.proname::text AS name,
                pg_catalog.pg_get_function_arguments(routine.oid) AS arguments,
                pg_catalog.pg_get_function_result(routine.oid) AS result,
                routine.proargnames AS names,
                (SELECT array_agg(namespace.nspname::text||'.'||kind.typname::text ORDER BY position)
                 FROM unnest(COALESCE(routine.proallargtypes,routine.proargtypes::oid[]))
                      WITH ORDINALITY AS argument(type_id,position)
                 JOIN pg_catalog.pg_type AS kind ON kind.oid=argument.type_id
                 JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid=kind.typnamespace) AS types,
                routine.proargmodes::text[] AS modes,
                routine.proallargtypes IS NOT NULL AS table_result,
                routine.proretset AS returns_set, routine.provolatile::text AS volatility,
                routine.prosrc AS body,
                (namespace.nspname='public' AND routine.prokind='f'
                 AND pg_catalog.pg_get_userbyid(routine.proowner)='postgres'
                 AND language.lanname=$3 AND routine.prosecdef=$5
                 AND routine.proconfig=$6::text[]
                 AND routine.pronargdefaults=0 AND routine.provariadic=0
                 AND NOT routine.proisstrict AND NOT routine.proleakproof
                 AND routine.proparallel='u' AND routine.prosupport=0
                 AND routine.probin IS NULL AND routine.prosqlbody IS NULL
                 AND routine.procost=100 AND routine.prorows=CASE WHEN routine.proretset THEN 1000 ELSE 0 END
                 AND pg_catalog.has_function_privilege('hephaestus_worker',routine.oid,'EXECUTE')=$2
                 AND pg_catalog.has_function_privilege('hephaestus_app',routine.oid,'EXECUTE')=$4) AS shape,
                (SELECT count(*)=1+(CASE WHEN $2 THEN 1 ELSE 0 END)+(CASE WHEN $4 THEN 1 ELSE 0 END)
                     AND count(*) FILTER (WHERE acl.grantee=routine.proowner)=1
                     AND count(*) FILTER (WHERE acl.grantee='hephaestus_worker'::regrole)=CASE WHEN $2 THEN 1 ELSE 0 END
                     AND count(*) FILTER (WHERE acl.grantee='hephaestus_app'::regrole)=CASE WHEN $4 THEN 1 ELSE 0 END
                     AND COALESCE(bool_and(
                     acl.grantor=routine.proowner AND NOT acl.is_grantable
                     AND acl.privilege_type='EXECUTE'
                     AND (acl.grantee=routine.proowner OR ($2 AND acl.grantee='hephaestus_worker'::regrole) OR ($4 AND acl.grantee='hephaestus_app'::regrole))),false)
                 FROM pg_catalog.aclexplode(COALESCE(routine.proacl,pg_catalog.acldefault('f',routine.proowner))) AS acl)
                    AS exact_acl
         FROM pg_catalog.pg_proc AS routine
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid=routine.pronamespace
         JOIN pg_catalog.pg_language AS language ON language.oid=routine.prolang
         WHERE routine.oid=pg_catalog.to_regprocedure($1)",
    ).bind(entry.signature).bind(entry.worker).bind(entry.language).bind(application_execute(entry.name)).bind(entry.name!="recipe_command_guard").bind(vec![if entry.name=="recipe_command_guard" {"search_path=pg_catalog, public"} else {"search_path=pg_catalog, public, pg_temp"}]).fetch_optional(&mut **admin).await?;
    if !observed.is_some_and(|routine| matches_routine(&routine, entry)) {
        return Err(invalid(&format!(
            "Source140 {} declaration, body or ACL differs",
            entry.name
        )));
    }
    Ok(())
}

fn matches_routine(routine: &Routine, entry: &Entry) -> bool {
    routine.name == entry.name
        && routine.arguments == entry.arguments
        && routine.result == entry.result
        && routine
            .names
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .eq(entry.names.iter().copied())
        && routine
            .types
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .eq(entry.types.iter().copied())
        && if entry.table_result {
            routine.modes.as_deref().is_some_and(|modes| {
                modes
                    .iter()
                    .map(String::as_str)
                    .eq(entry.modes.iter().copied())
            })
        } else {
            routine.modes.is_none()
        }
        && routine.table_result == entry.table_result
        && routine.returns_set == entry.returns_set
        && routine.volatility == entry.volatility
        && routine.body == source_body(entry.name)
        && routine.shape
        && routine.exact_acl
}

fn source_body(name: &str) -> &'static str {
    // These exact finite names cannot occur in another top-level declaration.
    let create = format!("FUNCTION {name}(");
    let source = if terminal143::contains(name) {
        terminal143::SOURCE
    } else if preclosure_drain::contains(name) {
        preclosure_drain::SOURCE
    } else {
        SOURCE
    };
    let (_, declaration) = source
        .split_once(&create)
        .expect("sealed Source140 declaration");
    let (_, body) = declaration
        .split_once(" AS $$")
        .expect("sealed dollar body");
    body.split_once("$$;").expect("sealed dollar end").0
}

async fn verify_triggers(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let hooks: [(&str, &str, &str, i16, bool, &str); 3] = [
        (
            "b_queued_invocation_cancel_inbox140",
            "command_inbox",
            "guard_queued_invocation_cancel_inbox140",
            7,
            false,
            "INSERT",
        ),
        (
            "ordinary_queued_cancel_inbox_commit140",
            "command_inbox",
            "validate_ordinary_queued_cancel_commit140",
            21,
            true,
            "INSERT OR UPDATE",
        ),
        (
            "ordinary_queued_cancel_run_commit140",
            "runs",
            "validate_ordinary_queued_cancel_commit140",
            17,
            true,
            "UPDATE",
        ),
    ];
    for (name, table, function, timing, deferred, event) in hooks {
        // PG17 trigger deparsing supplies the proper OLD/NEW namespace.
        let predicate = if table == "runs" {
            "((new.run_kind = 'invocation'::text) AND (new.cancel_requested_at IS NOT NULL))"
        } else {
            "(new.command_type = 'cancel_run'::text)"
        };
        let expected = format!(
            "CREATE {}TRIGGER {name} {} {event} ON public.{table} {}FOR EACH ROW WHEN ({predicate}) EXECUTE FUNCTION {function}()",
            if deferred { "CONSTRAINT " } else { "" },
            if deferred { "AFTER" } else { "BEFORE" },
            if deferred {
                "DEFERRABLE INITIALLY DEFERRED "
            } else {
                ""
            }
        );
        let exact: bool=sqlx::query_scalar(
            "SELECT count(*)=1 AND COALESCE(bool_and(
                 trigger.tgrelid=pg_catalog.to_regclass('public.'||$2)
                 AND trigger.tgfoid=pg_catalog.to_regprocedure('public.'||$3||'()')
                 AND trigger.tgtype=$4 AND trigger.tgenabled='O' AND NOT trigger.tgisinternal
                 AND trigger.tgnargs=0 AND octet_length(trigger.tgargs)=0
                 AND trigger.tgdeferrable=$5 AND trigger.tginitdeferred=$5
                 AND trigger.tgconstrrelid=0 AND trigger.tgconstrindid=0 AND trigger.tgparentid=0
                 AND (trigger.tgconstraint<>0)=$5
                 AND pg_catalog.pg_get_triggerdef(trigger.oid,false)=$6
                 AND (NOT $5 OR EXISTS(SELECT 1 FROM pg_catalog.pg_constraint AS constraint_record
                     WHERE constraint_record.oid=trigger.tgconstraint AND constraint_record.contype='t'
                       AND constraint_record.conname=trigger.tgname AND constraint_record.conrelid=trigger.tgrelid
                       AND constraint_record.connamespace='public'::regnamespace
                       AND constraint_record.condeferrable AND constraint_record.condeferred))),false)
             FROM pg_catalog.pg_trigger AS trigger
             WHERE trigger.tgname=$1",
        ).bind(name).bind(table).bind(function).bind(timing).bind(deferred).bind(expected)
        .fetch_one(&mut **admin).await?;
        if !exact {
            return Err(invalid(&format!(
                "Source140 {name} trigger contract differs"
            )));
        }
    }
    Ok(())
}

// These exact caller grants come from immutable121/129 and reviewed142/143.
fn application_execute(name: &str) -> bool {
    matches!(
        name,
        "load_existing_instance_closure118"
            | "remove_resume_context118"
            | "prepare_resumed_prepared_instance_closure142"
            | "inspect_prepared_instance_safety118"
            | "delete_prepared_instance143"
            | "load_prepared_instance_delete_continuation143"
            | "qualified_remove_resume_claim118"
            | "ready_retention_quiescent"
    )
}

#[cfg(test)]
mod tests {
    use super::{Routine, application_execute, entries, matches_routine, source_body};
    #[test]
    fn inherited_application_executors_remain_exact_and_finite() {
        for name in [
            "load_existing_instance_closure118",
            "remove_resume_context118",
            "prepare_resumed_prepared_instance_closure142",
            "inspect_prepared_instance_safety118",
            "delete_prepared_instance143",
            "load_prepared_instance_delete_continuation143",
            "qualified_remove_resume_claim118",
            "ready_retention_quiescent",
        ] {
            assert!(application_execute(name), "{name}");
        }
    }
    #[test]
    fn worker_private_unknown_and_public_names_are_not_application_executors() {
        for name in [
            "observe_prepared_instance_detach143",
            "observe_prepared_instance_deleted143",
            "prepared_instance_cleanup143",
            "unknown_routine",
            "PUBLIC",
        ] {
            assert!(!application_execute(name), "{name}");
        }
    }
    #[test]
    fn complete_match_still_denies_wrong_acl_shape_body_and_declaration() {
        let entry = entries::ENTRIES
            .iter()
            .find(|entry| entry.name == "load_existing_instance_closure118")
            .unwrap();
        let mut routine = Routine {
            name: entry.name.into(),
            arguments: entry.arguments.into(),
            result: entry.result.into(),
            names: Some(entry.names.iter().map(|name| (*name).into()).collect()),
            types: Some(entry.types.iter().map(|name| (*name).into()).collect()),
            modes: None,
            table_result: entry.table_result,
            returns_set: entry.returns_set,
            volatility: entry.volatility.into(),
            body: source_body(entry.name).into(),
            shape: true,
            exact_acl: true,
        };
        assert!(matches_routine(&routine, entry));
        routine.exact_acl = false;
        assert!(!matches_routine(&routine, entry));
        routine.exact_acl = true;
        routine.shape = false;
        assert!(!matches_routine(&routine, entry));
        routine.shape = true;
        routine.body.push(' ');
        assert!(!matches_routine(&routine, entry));
        routine.body = source_body(entry.name).into();
        routine.arguments.push_str(", foreign text");
        assert!(!matches_routine(&routine, entry));
    }
}
