// PG17 ordinary-table PK/UNIQUE/FK keys are non-inherited; CHECKs are checked separately.
use super::{Postgres, TABLE, Transaction, invalid};
type Key = (
    &'static str,
    &'static [i16],
    &'static str,
    &'static [&'static str],
    bool,
);
const KEYS: [Key; 24] = [
    ("p", &[1], "", &[], false),
    ("u", &[2], "", &[], false),
    ("f", &[2], "agent_instances", &["id"], false),
    ("f", &[3], "recipe_deployments", &["id"], false),
    ("f", &[4], "projects", &["id"], false),
    ("f", &[6], "agent_instance_revisions", &["id"], false),
    ("f", &[9], "recipe_effect_attempts", &["id"], false),
    ("u", &[10], "", &[], false),
    (
        "f",
        &[10],
        "agent_instance_removal_requests",
        &["id"],
        false,
    ),
    ("f", &[11], "recipe_effect_attempts", &["id"], false),
    ("u", &[12], "", &[], false),
    ("f", &[12], "recipe_execution_transitions", &["id"], false),
    ("u", &[13], "", &[], false),
    ("f", &[13], "recipe_effect_verifications", &["id"], false),
    ("u", &[14], "", &[], false),
    ("f", &[14], "recipe_effect_attempts", &["id"], false),
    ("f", &[24], "recipe_deployment_commands", &["id"], false),
    ("f", &[25], "users", &["id"], false),
    (
        "f",
        &[27],
        "recipe_remove_resume_admissions",
        &["command_id"],
        false,
    ),
    ("u", &[28], "", &[], false),
    ("f", &[28], "application_events", &["id"], true),
    ("u", &[31], "", &[], false),
    ("f", &[31], "agent_instance_events", &["id"], true),
    (
        "f",
        &[3, 5],
        "recipe_deployment_resources",
        &["deployment_id", "resource_name"],
        false,
    ),
];
pub(super) async fn verify(admin: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    for (kind, columns, foreign, foreign_columns, deferred) in KEYS {
        let exact: bool = sqlx::query_scalar("SELECT count(*)=1 AND COALESCE(bool_and(c.connamespace='public'::regnamespace AND c.condeferrable=$5 AND c.condeferred=$5 AND c.convalidated AND c.conislocal AND c.coninhcount=0 AND c.connoinherit AND c.conparentid=0 AND c.contypid=0 AND c.conbin IS NULL AND c.conindid<>0 AND CASE WHEN $1='f' THEN c.confrelid=pg_catalog.to_regclass('public.'||$3) AND c.confkey=ARRAY(SELECT a.attnum FROM unnest($4::text[]) WITH ORDINALITY n(name,position) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.confrelid AND a.attname=n.name ORDER BY n.position) AND c.confupdtype='a' AND c.confdeltype='a' AND c.confmatchtype='s' AND c.conindid=(SELECT indexrelid FROM pg_catalog.pg_index WHERE indrelid=c.confrelid AND indisprimary) ELSE c.confrelid=0 AND c.confkey IS NULL AND EXISTS(SELECT 1 FROM pg_catalog.pg_index i WHERE i.indexrelid=c.conindid AND i.indrelid=c.conrelid AND i.indisunique AND i.indisprimary=($1='p') AND i.indisvalid AND i.indisready AND i.indislive AND NOT i.indcheckxmin AND i.indexprs IS NULL AND i.indpred IS NULL AND i.indnatts=cardinality($2::smallint[]) AND i.indnkeyatts=i.indnatts AND ARRAY(SELECT unnest(i.indkey))= $2::smallint[]) END),false) FROM pg_catalog.pg_constraint c WHERE c.conrelid=pg_catalog.to_regclass($6) AND c.contype::text=$1 AND c.conkey=$2::smallint[]").bind(kind).bind(columns).bind(foreign).bind(foreign_columns).bind(deferred).bind(TABLE).fetch_one(&mut **admin).await?;
        if !exact {
            return Err(invalid("Source143 exact PK/UNIQUE/FK shape differs"));
        }
    }
    Ok(())
}
