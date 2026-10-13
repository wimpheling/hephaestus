use super::{Postgres, TABLE, Transaction, invalid};
#[path = "constraints/keys.rs"]
mod keys;
const CHECKS: [(&str, &str, &[i16]); 15] = [
    (
        "terminal143_id_non_nil",
        "CHECK ((id <> '00000000-0000-0000-0000-000000000000'::uuid))",
        &[1],
    ),
    (
        "terminal143_import_key",
        "CHECK ((octet_length(import_command_key) = 32))",
        &[7],
    ),
    (
        "terminal143_preparation_hash",
        "CHECK ((octet_length(preparation_hash) = 32))",
        &[8],
    ),
    (
        "terminal143_original_object",
        "CHECK ((jsonb_typeof(original_json) = 'object'::text))",
        &[15],
    ),
    (
        "terminal143_progress_object",
        "CHECK ((jsonb_typeof(delete_progress) = 'object'::text))",
        &[16],
    ),
    (
        "terminal143_configuration_hash_length",
        "CHECK ((octet_length(configuration_hash) = 32))",
        &[19],
    ),
    (
        "terminal143_before_version",
        "CHECK (((before_instance_version > 0) AND (before_instance_version < '9223372036854775807'::bigint)))",
        &[20],
    ),
    (
        "terminal143_after_version",
        "CHECK ((after_instance_version = (before_instance_version + 1)))",
        &[21, 20],
    ),
    (
        "terminal143_before_object",
        "CHECK ((jsonb_typeof(before_instance) = 'object'::text))",
        &[22],
    ),
    (
        "terminal143_after_object",
        "CHECK ((jsonb_typeof(after_instance) = 'object'::text))",
        &[23],
    ),
    (
        "terminal143_event_cursor",
        "CHECK ((event_cursor > 0))",
        &[29],
    ),
    (
        "terminal143_event_version",
        "CHECK ((event_aggregate_version > 0))",
        &[30],
    ),
    (
        "terminal143_configuration_hash",
        "CHECK ((configuration_hash = sha256(configuration_bytes)))",
        &[19, 17],
    ),
    (
        "terminal143_configuration_json",
        "CHECK ((configuration_json = (convert_from(configuration_bytes, 'UTF8'::name))::jsonb))",
        &[18, 17],
    ),
    (
        "terminal143_request_non_nil",
        "CHECK ((publisher_request_id <> '00000000-0000-0000-0000-000000000000'::uuid))",
        &[26],
    ),
];
pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_constraint WHERE conrelid=pg_catalog.to_regclass($1)",
    )
    .bind(TABLE)
    .fetch_one(&mut **admin)
    .await?;
    if count != 40 {
        return Err(invalid("Source143 exact 40 constraint inventory differs"));
    }
    for (name, definition, columns) in CHECKS {
        let exact: bool = sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(c.contype='c' AND c.connamespace='public'::regnamespace AND c.conkey=$3::smallint[] AND NOT c.condeferrable AND NOT c.condeferred AND c.convalidated AND c.conislocal AND c.coninhcount=0 AND NOT c.connoinherit AND c.conparentid=0 AND c.contypid=0 AND c.confrelid=0 AND c.conindid=0 AND c.confkey IS NULL AND c.conbin IS NOT NULL AND pg_catalog.pg_get_constraintdef(c.oid,false)=$2),false) FROM pg_catalog.pg_constraint c WHERE c.conrelid=pg_catalog.to_regclass($4) AND c.conname=$1").bind(name).bind(definition).bind(columns).bind(TABLE).fetch_one(&mut **admin).await?;
        if !exact {
            return Err(invalid(&format!("Source143 {name} exact CHECK differs")));
        }
    }
    keys::verify(admin).await
}
