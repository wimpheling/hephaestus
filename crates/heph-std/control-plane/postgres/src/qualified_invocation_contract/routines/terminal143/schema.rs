use super::{Postgres, Transaction, invalid};
#[path = "schema/columns.rs"]
mod columns;
#[path = "schema/constraints.rs"]
mod constraints;
#[path = "schema/hooks.rs"]
mod hooks;
const TABLE: &str = "public.recipe_prepared_instance_terminal_operations143";
pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let exact: bool = sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(c.relkind='r' AND c.relpersistence='p' AND c.relnatts=32 AND c.relchecks=15 AND c.relrowsecurity AND c.relforcerowsecurity AND pg_catalog.pg_get_userbyid(c.relowner)='postgres' AND c.relacl=ARRAY['postgres=arwdDxtm/postgres']::aclitem[] AND NOT c.relispartition AND c.relpartbound IS NULL AND c.reloptions IS NULL AND c.relam=2 AND c.reltablespace=0 AND c.relreplident='d' AND c.relhasindex AND c.relhastriggers AND c.reltype<>0 AND c.reltoastrelid<>0 AND NOT c.relhasrules AND NOT c.relhassubclass AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_policy p WHERE p.polrelid=c.oid) AND NOT pg_catalog.has_table_privilege('hephaestus_app',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER') AND NOT pg_catalog.has_table_privilege('hephaestus_worker',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')),false) FROM pg_catalog.pg_class c WHERE c.oid=pg_catalog.to_regclass($1)").bind(TABLE).fetch_one(&mut **admin).await?;
    if !exact {
        return Err(invalid(
            "Source143 table owner, ACL, RLS or native shape differs",
        ));
    }
    columns::verify(admin).await?;
    constraints::verify(admin).await?;
    hooks::verify(admin).await
}
