use super::{Postgres, TABLE, Transaction, invalid};
const COLUMNS: [(&str, &str, i16, bool, bool); 32] = [
    ("id", "uuid", 1_i16, true, false),
    ("instance_id", "uuid", 2_i16, true, false),
    ("deployment_id", "uuid", 3_i16, true, false),
    ("project_id", "uuid", 4_i16, true, false),
    ("resource_name", "text", 5_i16, true, false),
    ("revision_id", "uuid", 6_i16, true, false),
    ("import_command_key", "bytea", 7_i16, true, false),
    ("preparation_hash", "bytea", 8_i16, true, false),
    ("create_attempt_id", "uuid", 9_i16, true, false),
    ("removal_id", "uuid", 10_i16, true, false),
    ("drain_attempt_id", "uuid", 11_i16, true, false),
    ("detach_transition_id", "uuid", 12_i16, true, false),
    ("detach_verification_id", "uuid", 13_i16, true, false),
    ("delete_attempt_id", "uuid", 14_i16, true, false),
    ("original_json", "jsonb", 15_i16, true, false),
    ("delete_progress", "jsonb", 16_i16, true, false),
    ("configuration_bytes", "bytea", 17_i16, true, false),
    ("configuration_json", "jsonb", 18_i16, true, false),
    ("configuration_hash", "bytea", 19_i16, true, false),
    ("before_instance_version", "bigint", 20_i16, true, false),
    ("after_instance_version", "bigint", 21_i16, true, false),
    ("before_instance", "jsonb", 22_i16, true, false),
    ("after_instance", "jsonb", 23_i16, true, false),
    ("publisher_command_id", "uuid", 24_i16, true, false),
    ("publisher_actor_id", "uuid", 25_i16, true, false),
    ("publisher_request_id", "uuid", 26_i16, true, false),
    ("publisher_admission_id", "uuid", 27_i16, false, false),
    ("event_id", "uuid", 28_i16, true, false),
    ("event_cursor", "bigint", 29_i16, true, false),
    ("event_aggregate_version", "bigint", 30_i16, true, false),
    ("instance_event_id", "uuid", 31_i16, true, false),
    ("removed_at", "timestamp with time zone", 32_i16, true, true),
];
pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_attribute WHERE attrelid=pg_catalog.to_regclass($1) AND attnum>0").bind(TABLE).fetch_one(&mut **admin).await?;
    if count != 32 {
        return Err(invalid("Source143 column inventory differs"));
    }
    for (name, kind, ordinal, not_null, default) in COLUMNS {
        let exact: bool = sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(a.attnum=$3 AND pg_catalog.format_type(a.atttypid,a.atttypmod)=$2 AND a.atttypid=pg_catalog.to_regtype($2) AND a.atttypmod=-1 AND a.attndims=0 AND a.attnotnull=$4 AND a.atthasdef=$5 AND NOT a.atthasmissing AND NOT a.attisdropped AND a.attislocal AND a.attinhcount=0 AND a.attidentity='' AND a.attgenerated='' AND a.attcollation=CASE WHEN $2='text' THEN 'pg_catalog.default'::regcollation::oid ELSE 0::oid END AND a.attacl IS NULL AND a.attoptions IS NULL AND a.attfdwoptions IS NULL AND a.attmissingval IS NULL),false) FROM pg_catalog.pg_attribute a WHERE a.attrelid=pg_catalog.to_regclass($6) AND a.attname=$1").bind(name).bind(kind).bind(ordinal).bind(not_null).bind(default).bind(TABLE).fetch_one(&mut **admin).await?;
        if !exact {
            return Err(invalid(&format!(
                "Source143 {name} full attribute shape differs"
            )));
        }
    }
    let default: bool=sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(d.adnum=32 AND pg_catalog.pg_get_expr(d.adbin,d.adrelid,false)='now()'),false) FROM pg_catalog.pg_attrdef d WHERE d.adrelid=pg_catalog.to_regclass($1)").bind(TABLE).fetch_one(&mut **admin).await?;
    if !default {
        return Err(invalid("Source143 exact now() default differs"));
    }
    Ok(())
}
