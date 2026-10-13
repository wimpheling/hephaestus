use super::{Postgres, TABLE, Transaction, invalid};
pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    for (name, function, timing, deferred, event) in [
        (
            "aaa_prepared_instance_terminal_guard143",
            "guard_prepared_instance_terminal143",
            31_i16,
            false,
            "INSERT OR DELETE OR UPDATE",
        ),
        (
            "prepared_instance_terminal_commit143",
            "validate_prepared_instance_terminal143",
            5_i16,
            true,
            "INSERT",
        ),
    ] {
        let definition = format!(
            "CREATE {}TRIGGER {name} {} {event} ON {TABLE} {}FOR EACH ROW EXECUTE FUNCTION {function}()",
            if deferred { "CONSTRAINT " } else { "" },
            if deferred { "AFTER" } else { "BEFORE" },
            if deferred {
                "DEFERRABLE INITIALLY DEFERRED "
            } else {
                ""
            }
        );
        let exact: bool=sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(t.tgrelid=pg_catalog.to_regclass($6) AND t.tgfoid=pg_catalog.to_regprocedure('public.'||$2||'()') AND t.tgtype=$3 AND t.tgenabled='O' AND NOT t.tgisinternal AND t.tgnargs=0 AND octet_length(t.tgargs)=0 AND t.tgqual IS NULL AND t.tgattr=''::int2vector AND t.tgoldtable IS NULL AND t.tgnewtable IS NULL AND pg_catalog.pg_get_triggerdef(t.oid,false)=$5 AND t.tgdeferrable=$4 AND t.tginitdeferred=$4 AND t.tgconstrrelid=0 AND t.tgconstrindid=0 AND t.tgparentid=0 AND (t.tgconstraint<>0)=$4 AND (NOT $4 OR EXISTS(SELECT 1 FROM pg_catalog.pg_constraint c WHERE c.oid=t.tgconstraint AND c.contype='t' AND c.conname=t.tgname AND c.conrelid=t.tgrelid AND c.connamespace='public'::regnamespace AND c.condeferrable AND c.condeferred))),false) FROM pg_catalog.pg_trigger t WHERE t.tgname=$1").bind(name).bind(function).bind(timing).bind(deferred).bind(definition).bind(TABLE).fetch_one(&mut **admin).await?;
        if !exact {
            return Err(invalid("Source143 immediate/deferred trigger differs"));
        }
    }
    let exact: bool=sqlx::query_scalar("SELECT count(*)=64 AND COALESCE(bool_and(t.tgenabled='O' AND t.tgisinternal AND t.tgnargs=0 AND octet_length(t.tgargs)=0 AND t.tgqual IS NULL AND t.tgattr=''::int2vector AND t.tgparentid=0 AND t.tgoldtable IS NULL AND t.tgnewtable IS NULL AND t.tgdeferrable=c.condeferrable AND t.tginitdeferred=c.condeferred AND t.tgconstrindid=c.conindid AND ((t.tgfoid=1644 AND t.tgtype=5 AND t.tgrelid=c.conrelid AND t.tgconstrrelid=c.confrelid AND t.tgname='RI_ConstraintTrigger_c_'||t.oid::text) OR (t.tgfoid=1645 AND t.tgtype=17 AND t.tgrelid=c.conrelid AND t.tgconstrrelid=c.confrelid AND t.tgname='RI_ConstraintTrigger_c_'||t.oid::text) OR (t.tgfoid=1654 AND t.tgtype=9 AND t.tgrelid=c.confrelid AND t.tgconstrrelid=c.conrelid AND t.tgname='RI_ConstraintTrigger_a_'||t.oid::text) OR (t.tgfoid=1655 AND t.tgtype=17 AND t.tgrelid=c.confrelid AND t.tgconstrrelid=c.conrelid AND t.tgname='RI_ConstraintTrigger_a_'||t.oid::text))),false) AND count(DISTINCT(t.tgconstraint,t.tgfoid))=64 FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_constraint c ON c.oid=t.tgconstraint WHERE c.conrelid=pg_catalog.to_regclass($1) AND c.contype='f'").bind(TABLE).fetch_one(&mut **admin).await?;
    if !exact {
        return Err(invalid("Source143 complete RI trigger graph differs"));
    }
    Ok(())
}
