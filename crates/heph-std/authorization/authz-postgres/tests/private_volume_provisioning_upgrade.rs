//! Isolated 0102 -> 0103 upgrade of real prior standalone and legacy rows.

use serde_json::{Value, json};
use sqlx::{PgPool, migrate::Migrator};
use std::borrow::Cow;
use uuid::Uuid;

#[path = "postgres/seed.rs"]
mod seed;
// Reuse the canonical relational fixture; unrelated authorization cases are unused.
#[allow(dead_code)]
#[path = "postgres/support.rs"]
mod support;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../migrations");

#[derive(sqlx::FromRow)]
struct StandaloneIntentRow {
    project_id: Uuid,
    instance_id: Option<Uuid>,
    capacity_bytes: i64,
    host_id: Option<String>,
    host_path: Option<String>,
    filesystem_uuid: Option<Uuid>,
    provisioning_state: String,
    provisioning_generation: i64,
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn previous_standalone_and_legacy_provider_intents_survive_upgrade(pool: PgPool) {
    checkpoint(102)
        .run(&pool)
        .await
        .expect("apply schema through0102");
    let fixture = seed::seed(&pool).await;
    let standalone = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes) VALUES($1,$2,'uninitialized',16777216)")
        .bind(standalone).bind(fixture.project).execute(&pool).await.expect("historical standalone without UUID or host handles");
    let smaller = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes) VALUES($1,$2,'uninitialized',1)")
        .bind(smaller).bind(fixture.project).execute(&pool).await.expect("historical below-provider capacity remains durable");
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=13,key_reference='preserved-key',encryption_version=2,backup_revision=7,checksum='preserved-checksum' WHERE id=$1")
        .bind(fixture.volume).execute(&pool).await.expect("nondefault existing legacy metadata");
    let lease = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',13,'active',now()+interval '1 hour')")
        .bind(lease).bind(fixture.volume).bind(fixture.instance).bind(fixture.run).execute(&pool).await.expect("existing legacy lease fence");
    let before = legacy_snapshot(&pool, fixture.volume, lease).await;
    let historical_checksums = checksums(&pool).await;
    let indexes = lease_indexes(&pool).await;
    checkpoint(103)
        .run(&pool)
        .await
        .expect("upgrade populated database to0103");
    assert_eq!(
        legacy_snapshot(&pool, fixture.volume, lease).await,
        before,
        "old UUID/capacity/provider handles and lease fence remain exact"
    );
    assert_eq!(
        checksums(&pool).await,
        historical_checksums,
        "applied0101/0102 and older checksums remain exact"
    );
    assert_eq!(
        lease_indexes(&pool).await,
        indexes,
        "lease uniqueness and FK remain unchanged"
    );
    let row: StandaloneIntentRow = sqlx::query_as("SELECT project_id,instance_id,capacity_bytes,host_id,host_path,filesystem_uuid,provisioning_state,provisioning_generation FROM agent_instance_state_volumes WHERE id=$1")
        .bind(standalone).fetch_one(&pool).await.expect("historical standalone projection");
    assert_eq!(row.project_id, fixture.project);
    assert_eq!(row.instance_id, None);
    assert_eq!(row.capacity_bytes, 16_777_216);
    assert_eq!(row.host_id, None);
    assert_eq!(row.host_path, None);
    let uuid = row
        .filesystem_uuid
        .expect("UUID reserved once only for unassigned standalone");
    assert!(!uuid.is_nil());
    assert_eq!(row.provisioning_state, "reserved");
    assert_eq!(row.provisioning_generation, 0);
    checkpoint(103)
        .run(&pool)
        .await
        .expect("idempotent migration replay");
    let replay: Uuid =
        sqlx::query_scalar("SELECT filesystem_uuid FROM agent_instance_state_volumes WHERE id=$1")
            .bind(standalone)
            .fetch_one(&pool)
            .await
            .expect("stable UUID replay");
    assert_eq!(replay, uuid);
    let retained_small: i64 =
        sqlx::query_scalar("SELECT capacity_bytes FROM agent_instance_state_volumes WHERE id=$1")
            .bind(smaller)
            .fetch_one(&pool)
            .await
            .expect("historical capacity");
    assert_eq!(retained_small, 1);
    let error = sqlx::query(
        "UPDATE agent_instance_state_volumes SET filesystem_uuid=gen_random_uuid() WHERE id=$1",
    )
    .bind(standalone)
    .execute(&pool)
    .await
    .expect_err("UUID intent immutable after upgrade");
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    println!(
        "REAL_VOLUME_PROVISIONING_UPGRADE=1 historical_schema=102 checkpoint_schema=103 old_checksums_preserved=1 legacy_handles_uuid_capacity_lease_fence_preserved=1 unassigned_standalone_uuid_reserved_once=1 below_provider_capacity_preserved=1 database_only=1"
    );
}

fn checkpoint(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

async fn legacy_snapshot(pool: &PgPool, volume: Uuid, lease: Uuid) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('volume',to_jsonb(volume)-'provisioning_state'-'provisioning_generation','lease',to_jsonb(lease)) FROM agent_instance_state_volumes volume JOIN agent_instance_volume_leases lease ON lease.volume_id=volume.id WHERE volume.id=$1 AND lease.id=$2")
        .bind(volume).bind(lease).fetch_one(pool).await.expect("legacy snapshot")
}

async fn checksums(pool: &PgPool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=102 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("old checksums")
}

async fn lease_indexes(pool: &PgPool) -> Value {
    let definitions: Vec<(String, String)> = sqlx::query_as("SELECT indexname,indexdef FROM pg_indexes WHERE schemaname='public' AND tablename='agent_instance_volume_leases' ORDER BY indexname").fetch_all(pool).await.expect("lease indexes");
    let constraints: Vec<(String, String)> = sqlx::query_as("SELECT conname,pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid='agent_instance_volume_leases'::regclass ORDER BY conname").fetch_all(pool).await.expect("lease constraints");
    json!({"indexes": definitions, "constraints": constraints})
}
