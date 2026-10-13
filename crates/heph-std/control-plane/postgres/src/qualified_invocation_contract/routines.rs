#[path = "routines/queued_cancel.rs"]
mod queued_cancel;
#[path = "routines/retained_reuse.rs"]
mod retained_reuse;

use super::{Postgres, Transaction, invalid};
use sqlx::FromRow;

const HOOKS: [&str; 7] = [
    "advance_owned_observation_head",
    "guard_owned_observation_head",
    "guard_owned_partial_receipt",
    "guard_recipe_owned_volume_preparation",
    "reject_instance_activation_rewrite",
    "reject_retired_owned_birth_progress",
    "verify_partial_birth_retirement_commit",
];
const SAFE_PATH: &str = "search_path=pg_catalog, public, pg_temp";

// Independent SQL catalog and privilege facts represent separate checks.
#[allow(clippy::struct_excessive_bools)]
#[derive(FromRow)]
struct Hook {
    name: String,
    owner: String,
    kind: String,
    arguments: i16,
    trigger_result: bool,
    plpgsql: bool,
    security_definer: bool,
    settings: Option<Vec<String>>,
    owner_only_acl: bool,
    worker_execute: bool,
    application_execute: bool,
}

pub(super) async fn verify_hooks(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let hooks: Vec<Hook> = sqlx::query_as(
        "SELECT routine.proname::text AS name,
                pg_catalog.pg_get_userbyid(routine.proowner)::text AS owner,
                routine.prokind::text AS kind, routine.pronargs AS arguments,
                routine.prorettype = 'pg_catalog.trigger'::regtype AS trigger_result,
                language.lanname = 'plpgsql' AS plpgsql,
                routine.prosecdef AS security_definer, routine.proconfig AS settings,
                (SELECT count(*) = 1 AND COALESCE(bool_and(
                     entry.grantee = routine.proowner AND entry.grantor = routine.proowner
                     AND entry.privilege_type = 'EXECUTE' AND NOT entry.is_grantable), false)
                 FROM pg_catalog.aclexplode(COALESCE(
                     routine.proacl, pg_catalog.acldefault('f', routine.proowner))) AS entry)
                    AS owner_only_acl,
                pg_catalog.has_function_privilege('hephaestus_worker', routine.oid, 'EXECUTE')
                    AS worker_execute,
                pg_catalog.has_function_privilege('hephaestus_app', routine.oid, 'EXECUTE')
                    AS application_execute
         FROM pg_catalog.pg_proc AS routine
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = routine.pronamespace
         JOIN pg_catalog.pg_language AS language ON language.oid = routine.prolang
         WHERE namespace.nspname = 'public' AND routine.proname::text = ANY($1::text[])",
    )
    .bind(HOOKS.as_slice())
    .fetch_all(&mut **admin)
    .await?;
    if hooks.len() != HOOKS.len()
        || HOOKS
            .iter()
            .any(|name| !hooks.iter().any(|hook| hook.name == *name))
    {
        return Err(invalid(
            "Source138 requires exactly seven public zero-argument hooks",
        ));
    }
    for hook in hooks {
        // Source138 hardens the six SECURITY DEFINER hooks. The activation
        // hook is SECURITY INVOKER and already has this same Source119 path.
        if hook.owner != "postgres"
            || hook.kind != "f"
            || hook.arguments != 0
            || !hook.trigger_result
            || !hook.plpgsql
            || hook.security_definer != (hook.name != "reject_instance_activation_rewrite")
            || !hook
                .settings
                .as_ref()
                .is_some_and(|settings| settings.len() == 1 && settings[0] == SAFE_PATH)
            || !hook.owner_only_acl
            || hook.worker_execute
            || hook.application_execute
        {
            return Err(invalid(&format!(
                "Source138 hook {} shape, owner ACL or exact search path differs",
                hook.name,
            )));
        }
    }
    Ok(())
}

// Finite interfaces used by qualified admission, RunPG/volume execution,
// authority, saved-empty secrets and cleanup. These checks never invoke helpers
// or replace their live per-transaction authorization and identity checks.
// Columns: public signature, exact PostgreSQL result declaration, worker access.
const HELPERS: [(&str, &str, bool); 21] = [
    (
        "public.assert_retained_owned_reuse_context(uuid,uuid,bigint,bytea,text,text,uuid)",
        "void",
        true,
    ),
    (
        "public.qualified_instance_invocation_context(uuid)",
        "TABLE(original_import_key bytea, release_agent uuid, first_mutex integer, second_mutex integer)",
        false,
    ),
    (
        "public.write_qualified_instance_invocation(uuid,bytea,uuid,uuid,uuid,uuid,bytea,text)",
        "TABLE(invocation uuid, instance uuid, revision uuid, run uuid, start_command uuid)",
        false,
    ),
    (
        "public.consume_qualified_invocation119(uuid,jsonb,bytea,text,text,text)",
        "SETOF runs",
        true,
    ),
    (
        "public.read_qualified_invocation119(uuid,jsonb,bytea)",
        "SETOF runs",
        true,
    ),
    (
        "public.load_qualified_invocation119_selections(uuid,bytea)",
        "jsonb",
        true,
    ),
    (
        "public.lock_qualified_invocation119(uuid,bytea)",
        "runs",
        true,
    ),
    (
        "public.acquire_qualified_invocation119(uuid,bytea,jsonb,text,timestamptz,timestamptz)",
        "void",
        true,
    ),
    (
        "public.refresh_qualified_invocation119(uuid,bytea,jsonb,text,timestamptz,timestamptz,boolean)",
        "void",
        true,
    ),
    (
        "public.verify_invocation119_pre_acquisition(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,bigint,text)",
        "boolean",
        true,
    ),
    (
        "public.verify_invocation119_empty_secrets(uuid,bytea,uuid,uuid,uuid,uuid,uuid,boolean,text,text)",
        "boolean",
        true,
    ),
    (
        "public.verify_invocation119_empty_secret_cleanup(uuid,bytea,text,text)",
        "jsonb",
        true,
    ),
    (
        "public.read_invocation119_authority_context(uuid,bytea,boolean)",
        "TABLE(actor_id uuid, request_id uuid, instance_id uuid, revision_id uuid, release_id uuid, release_agent_id uuid, command_id uuid)",
        true,
    ),
    (
        "public.write_invocation119_session(uuid,bytea,text,bytea,bytea,bigint,bytea,timestamptz,timestamptz,text,text)",
        "SETOF runtime_authority_sessions",
        true,
    ),
    (
        "public.find_invocation119_session(uuid,bytea)",
        "SETOF runtime_authority_sessions",
        true,
    ),
    (
        "public.acknowledge_invocation119_session(uuid,bytea,bigint,timestamptz)",
        "SETOF runtime_authority_sessions",
        true,
    ),
    (
        "public.invocation119_typed_mount_authorized(uuid,bytea,uuid,uuid,uuid,text,uuid,text,bytea)",
        "boolean",
        true,
    ),
    (
        "public.load_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea)",
        "SETOF runs",
        true,
    ),
    (
        "public.lock_run_cleanup_wake118(uuid,uuid,jsonb,text,text,bytea)",
        "void",
        true,
    ),
    (
        "public.discover_queued_run_cleanup_context119(uuid,text,text,bytea)",
        "TABLE(instance_id uuid, original jsonb)",
        true,
    ),
    (
        "public.lock_queued_cancel_delivery119(uuid,jsonb,text,text,bytea)",
        "void",
        true,
    ),
];

pub(super) async fn verify_permissions(
    admin: &mut Transaction<'_, Postgres>,
    worker: &mut Transaction<'_, Postgres>,
    application: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    retained_reuse::verify(admin).await?;
    queued_cancel::verify(admin).await?;
    check_schema_access(worker).await?;
    check_schema_access(application).await?;
    for (signature, result, worker_access) in HELPERS {
        let shape: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_catalog.pg_proc AS routine
                JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = routine.pronamespace
                WHERE routine.oid = pg_catalog.to_regprocedure($1)
                  AND namespace.nspname = 'public' AND routine.prokind = 'f'
                  AND pg_catalog.pg_get_userbyid(routine.proowner) = 'postgres'
                  AND pg_catalog.pg_get_function_result(routine.oid) = $2
            )",
        )
        .bind(signature)
        .bind(result)
        .fetch_one(&mut **admin)
        .await?;
        if !shape {
            return Err(invalid(&format!(
                "qualified helper {signature} identity, owner or result shape differs",
            )));
        }
        check_execute(worker, signature, worker_access).await?;
        check_execute(application, signature, !worker_access).await?;
    }
    Ok(())
}

async fn check_schema_access(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    let schema_access: bool = sqlx::query_scalar(
        "SELECT pg_catalog.has_schema_privilege(current_user, 'public', 'USAGE')
                AND NOT pg_catalog.has_schema_privilege(current_user, 'public', 'CREATE')",
    )
    .fetch_one(&mut **transaction)
    .await?;
    if !schema_access {
        return Err(invalid(
            "application and worker need public USAGE without CREATE",
        ));
    }
    Ok(())
}

async fn check_execute(
    transaction: &mut Transaction<'_, Postgres>,
    signature: &str,
    expected: bool,
) -> Result<(), sqlx::Error> {
    let access: Option<bool> = sqlx::query_scalar(
        "SELECT pg_catalog.has_function_privilege(
            current_user, pg_catalog.to_regprocedure($1), 'EXECUTE')",
    )
    .bind(signature)
    .fetch_one(&mut **transaction)
    .await?;
    if access != Some(expected) {
        return Err(invalid(&format!(
            "actual role helper execution permission differs for {signature}",
        )));
    }
    Ok(())
}
