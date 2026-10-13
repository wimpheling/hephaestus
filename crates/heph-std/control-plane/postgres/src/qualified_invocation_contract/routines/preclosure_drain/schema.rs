use super::{Postgres, Transaction, invalid};

pub(super) async fn verify_schema(
    admin: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    let exact: bool = sqlx::query_scalar(
        "SELECT
           (SELECT count(*)=1 AND COALESCE(bool_and(a.attnum=9 AND a.atttypid='uuid'::regtype
             AND a.atttypmod=-1 AND a.attndims=0 AND NOT a.attnotnull AND NOT a.atthasdef
             AND NOT a.atthasmissing AND NOT a.attisdropped AND a.attislocal AND a.attinhcount=0
             AND a.attidentity='' AND a.attgenerated='' AND a.attcollation=0 AND a.attacl IS NULL
             AND a.attoptions IS NULL AND a.attfdwoptions IS NULL AND a.attmissingval IS NULL),false)
            FROM pg_catalog.pg_attribute a WHERE a.attrelid='public.agent_instance_removal_requests'::regclass
             AND a.attname='prepared_resume_command_id')
           AND (SELECT count(*)=1 AND COALESCE(bool_and(c.contype='f' AND c.connamespace='public'::regnamespace
             AND c.conrelid='public.agent_instance_removal_requests'::regclass
             AND c.confrelid='public.recipe_remove_resume_admissions'::regclass
             AND c.conkey=ARRAY[9]::smallint[] AND c.confkey=ARRAY[1]::smallint[]
             AND c.confupdtype='a' AND c.confdeltype='a' AND c.confmatchtype='s'
             AND NOT c.condeferrable AND NOT c.condeferred AND c.convalidated AND c.conislocal
             AND c.coninhcount=0 AND c.connoinherit AND c.conparentid=0 AND c.contypid=0
             AND c.conbin IS NULL AND c.conindid=(SELECT indexrelid FROM pg_catalog.pg_index
                WHERE indrelid='public.recipe_remove_resume_admissions'::regclass AND indisprimary)),false)
            FROM pg_catalog.pg_constraint c WHERE c.conname='instance_removal_prepared_resume_command142_fkey')
           AND (SELECT count(*)=1 AND COALESCE(bool_and(c.contype='c' AND c.connamespace='public'::regnamespace
             AND c.conrelid='public.agent_instance_removal_requests'::regclass AND c.conkey=ARRAY[9]::smallint[]
             AND NOT c.condeferrable AND NOT c.condeferred AND c.convalidated AND c.conislocal
             AND c.coninhcount=0 AND NOT c.connoinherit AND c.conparentid=0 AND c.contypid=0
             AND pg_catalog.pg_get_constraintdef(c.oid,false)=$1),false)
            FROM pg_catalog.pg_constraint c WHERE c.conname='instance_removal_prepared_resume_command142_non_nil')
           AND (SELECT count(*)=1 AND COALESCE(bool_and(c.contype='c' AND c.connamespace='public'::regnamespace
             AND c.conrelid='public.recipe_deployment_commands'::regclass AND c.conkey=ARRAY[9]::smallint[]
             AND NOT c.condeferrable AND NOT c.condeferred AND c.convalidated AND c.conislocal
             AND c.coninhcount=0 AND NOT c.connoinherit AND c.conparentid=0 AND c.contypid=0
             AND pg_catalog.pg_get_constraintdef(c.oid,false)=$2),false)
            FROM pg_catalog.pg_constraint c WHERE c.conname='recipe_deployment_commands_receipt_lifecycle_check')")
        .bind("CHECK (((prepared_resume_command_id IS NULL) OR (prepared_resume_command_id <> '00000000-0000-0000-0000-000000000000'::uuid)))")
        .bind("CHECK ((receipt_lifecycle = ANY (ARRAY['installing'::text, 'removing'::text, 'removed'::text, 'recovery_required'::text])))")
        .fetch_one(&mut **admin).await?;
    if !exact {
        return Err(invalid("Source142 column, FK or CHECK differs"));
    }
    for (name, function, timing, deferred) in [
        (
            "aaa_prepared_resume_origin142",
            "guard_prepared_resume_origin142",
            7_i16,
            false,
        ),
        (
            "prepared_resume_origin_commit142",
            "validate_prepared_resume_origin142",
            5_i16,
            true,
        ),
    ] {
        let expected = format!(
            "CREATE {}TRIGGER {name} {} INSERT ON public.agent_instance_removal_requests {}FOR EACH ROW EXECUTE FUNCTION {function}()",
            if deferred { "CONSTRAINT " } else { "" },
            if deferred { "AFTER" } else { "BEFORE" },
            if deferred {
                "DEFERRABLE INITIALLY DEFERRED "
            } else {
                ""
            }
        );
        let exact: bool = sqlx::query_scalar(
            "SELECT count(*)=1 AND COALESCE(bool_and(t.tgrelid='public.agent_instance_removal_requests'::regclass
             AND t.tgfoid=pg_catalog.to_regprocedure('public.'||$2||'()') AND t.tgtype=$3
             AND t.tgenabled='O' AND NOT t.tgisinternal AND t.tgnargs=0 AND octet_length(t.tgargs)=0
             AND t.tgqual IS NULL AND t.tgattr=''::int2vector AND t.tgoldtable IS NULL AND t.tgnewtable IS NULL
             AND pg_catalog.pg_get_triggerdef(t.oid,false)=$5 AND t.tgdeferrable=$4 AND t.tginitdeferred=$4
             AND t.tgconstrrelid=0 AND t.tgconstrindid=0 AND t.tgparentid=0 AND (t.tgconstraint<>0)=$4
             AND (NOT $4 OR EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c WHERE c.oid=t.tgconstraint
               AND c.contype='t' AND c.conname=t.tgname AND c.conrelid=t.tgrelid
               AND c.connamespace='public'::regnamespace AND c.condeferrable AND c.condeferred))),false)
             FROM pg_catalog.pg_trigger t WHERE t.tgname=$1")
            .bind(name).bind(function).bind(timing).bind(deferred).bind(expected).fetch_one(&mut **admin).await?;
        if !exact {
            return Err(invalid(
                "Source142 ordered immediate/deferred trigger differs",
            ));
        }
    }
    // Names establish PG's BEFORE trigger order, and the original reservation guard must survive.
    let ordered: bool=sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(t.tgtype=7 AND t.tgenabled='O' AND NOT t.tgisinternal AND t.tgfoid='public.guard_prepared_instance_removal_order118()'::regprocedure),false) FROM pg_catalog.pg_trigger t WHERE t.tgrelid='public.agent_instance_removal_requests'::regclass AND t.tgname='aab_prepared_instance_removal_order118'")
        .fetch_one(&mut **admin).await?;
    if !ordered {
        return Err(invalid("Source142 original reservation trigger differs"));
    }
    Ok(())
}
