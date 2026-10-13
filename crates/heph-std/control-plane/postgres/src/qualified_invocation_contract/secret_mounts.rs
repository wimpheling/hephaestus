//! Finite Source141 ordinary worker metadata ACL; no runtime authority is issued.

use super::{Postgres, Transaction, invalid};
use sqlx::FromRow;

const PRIVILEGES: [&str; 8] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "TRUNCATE",
    "REFERENCES",
    "TRIGGER",
    "MAINTAIN",
];

#[derive(FromRow)]
struct MountTable {
    owner: String,
    kind: String,
    original_shape: bool,
    exact_acl: bool,
}

pub(super) async fn verify(
    admin: &mut Transaction<'_, Postgres>,
    worker: &mut Transaction<'_, Postgres>,
    application: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    let table: Option<MountTable> = sqlx::query_as(
        "WITH mount AS (
             SELECT relation.* FROM pg_catalog.pg_class AS relation
             JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
             WHERE namespace.nspname = 'public' AND relation.relname = 'secret_runtime_mounts'
         ), actual_acl AS (
             SELECT entry.grantor, entry.grantee, entry.privilege_type, entry.is_grantable
             FROM mount, pg_catalog.aclexplode(COALESCE(
                 mount.relacl, pg_catalog.acldefault('r', mount.relowner))) AS entry
         ), expected_acl AS (
             SELECT entry.grantor, entry.grantee, entry.privilege_type, entry.is_grantable
             FROM mount, pg_catalog.aclexplode(pg_catalog.acldefault('r', mount.relowner)) AS entry
             UNION ALL
             SELECT mount.relowner, 'hephaestus_worker'::regrole::oid, entry.privilege, false
             FROM mount, unnest(ARRAY['SELECT','INSERT','UPDATE']::text[]) AS entry(privilege)
         )
         SELECT pg_catalog.pg_get_userbyid(mount.relowner)::text AS owner,
                mount.relkind::text AS kind,
                (NOT mount.relrowsecurity AND NOT mount.relforcerowsecurity
                 AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_attribute AS attribute
                     WHERE attribute.attrelid = mount.oid AND attribute.attacl IS NOT NULL))
                    AS original_shape,
                (NOT EXISTS (SELECT * FROM actual_acl EXCEPT SELECT * FROM expected_acl)
                 AND NOT EXISTS (SELECT * FROM expected_acl EXCEPT SELECT * FROM actual_acl))
                    AS exact_acl
         FROM mount",
    )
    .fetch_optional(&mut **admin)
    .await?;
    if !table.is_some_and(|table| {
        table.owner == "postgres" && table.kind == "r" && table.original_shape && table.exact_acl
    }) {
        return Err(invalid(
            "Source141 secret mount metadata owner, shape or exact ACL differs",
        ));
    }
    check_privileges(worker, &["SELECT", "INSERT", "UPDATE"]).await?;
    check_privileges(application, &[]).await?;
    // This executes the actual worker SELECT without returning mount identifiers.
    let absent: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT opaque_directory FROM public.secret_runtime_mounts WHERE FALSE")
            .fetch_optional(&mut **worker)
            .await?;
    if absent.is_some() {
        return Err(invalid("Source141 zero-row worker metadata read differs"));
    }
    Ok(())
}

async fn check_privileges(
    transaction: &mut Transaction<'_, Postgres>,
    expected: &[&str],
) -> Result<(), sqlx::Error> {
    let observed: Vec<String> = sqlx::query_scalar(
        "SELECT privilege FROM unnest($1::text[]) WITH ORDINALITY AS entry(privilege, ordinal)
         WHERE pg_catalog.has_table_privilege(current_user, 'public.secret_runtime_mounts', privilege)
         ORDER BY ordinal",
    )
    .bind(PRIVILEGES.as_slice())
    .fetch_all(&mut **transaction)
    .await?;
    if observed.iter().map(String::as_str).collect::<Vec<_>>() != expected {
        return Err(invalid(
            "Source141 actual role secret mount metadata permissions differ",
        ));
    }
    Ok(())
}
