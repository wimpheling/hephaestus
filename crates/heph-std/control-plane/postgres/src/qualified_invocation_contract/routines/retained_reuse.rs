//! Finite Source139 metadata; this observation grants no retained reuse authority.
use super::super::{Postgres, Transaction, invalid};

pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS (
           SELECT 1 FROM pg_catalog.pg_proc AS routine
           JOIN pg_catalog.pg_language AS language ON language.oid = routine.prolang
           WHERE routine.oid = pg_catalog.to_regprocedure(
             'public.assert_retained_owned_reuse_context(uuid,uuid,bigint,bytea,text,text,uuid)')
             AND pg_catalog.pg_get_userbyid(routine.proowner) = 'postgres'
             AND routine.prokind = 'f' AND routine.prosecdef AND NOT routine.proretset
             AND routine.pronargs = 7 AND routine.pronargdefaults = 0
             AND routine.provariadic = 0 AND routine.proallargtypes IS NULL
             AND routine.proargmodes IS NULL
             AND routine.proargnames = ARRAY['p_closure','p_operation','p_generation',
               'p_purpose','p_namespace','p_host','p_retention_operation']::text[]
             AND routine.prorettype = 'pg_catalog.void'::regtype
             AND ARRAY(SELECT argument FROM pg_catalog.unnest(routine.proargtypes::oid[])
               WITH ORDINALITY AS arguments(argument, position) ORDER BY position) = ARRAY[
               'pg_catalog.uuid'::regtype::oid, 'pg_catalog.uuid'::regtype::oid,
               'pg_catalog.int8'::regtype::oid, 'pg_catalog.bytea'::regtype::oid,
               'pg_catalog.text'::regtype::oid, 'pg_catalog.text'::regtype::oid,
               'pg_catalog.uuid'::regtype::oid]
             AND language.lanname = 'plpgsql'
             AND routine.proconfig = ARRAY['search_path=pg_catalog, public, pg_temp']::text[]
             AND (SELECT count(*) = 2
                 AND count(*) FILTER (WHERE entry.grantee = routine.proowner) = 1
                 AND count(*) FILTER (WHERE entry.grantee = 'hephaestus_worker'::regrole::oid) = 1
                 AND COALESCE(bool_and(
                 entry.grantee IN (routine.proowner, 'hephaestus_worker'::regrole::oid)
                 AND entry.grantor = routine.proowner AND entry.privilege_type = 'EXECUTE'
                 AND NOT entry.is_grantable), false)
               FROM pg_catalog.aclexplode(routine.proacl) AS entry)
        )",
    )
    .fetch_one(&mut **admin)
    .await?;
    if !valid {
        return Err(invalid(
            "Source139 retained reuse helper shape, owner, safe path or worker-only ACL differs",
        ));
    }
    Ok(())
}
