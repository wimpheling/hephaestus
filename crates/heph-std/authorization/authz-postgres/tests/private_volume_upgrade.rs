//! Real isolated `PostgreSQL` rehearsal from schema 0100 to checkpoint 0102.
//!
//! This checks durable rows and backing-file references, not backing-file bytes.
//! `SQLx` owns the disposable database lifecycle; set `DATABASE_URL` explicitly.

use std::borrow::Cow;

use serde_json::{Value, json};
use sqlx::{PgPool, migrate::Migrator};
use uuid::Uuid;

#[path = "postgres/seed.rs"]
mod seed;
// Reuse the historical relational fixture; this target needs only its volume graph.
#[allow(dead_code)]
#[path = "postgres/support.rs"]
mod support;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../migrations");
const FENCE: i64 = 13;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn historical_private_volume_upgrade_preserves_execution_evidence(pool: PgPool) {
    let historical = checkpoint(100);
    historical
        .run(&pool)
        .await
        .expect("apply only historical migrations through 0100");
    let maximum: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("historical schema version");
    assert_eq!(
        maximum, 100,
        "fixture must be seeded before private-volume migrations"
    );
    let owner_column: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema = 'public'
         AND table_name = 'agent_instance_state_volumes' AND column_name = 'project_id')",
    )
    .fetch_one(&pool)
    .await
    .expect("historical volume columns");
    assert!(
        !owner_column,
        "legacy schema has no standalone volume owner column"
    );
    let fixture = seed::seed(&pool).await;
    let attempt = seed_execution_evidence(&pool, &fixture).await;
    let before = snapshot(&pool, &fixture, attempt).await;
    let checksums = migration_checksums(&pool).await;
    let indexes = execution_indexes(&pool).await;

    checkpoint(102)
        .run(&pool)
        .await
        .expect("upgrade populated historical database through checkpoint 0102");
    let maximum: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("upgraded checkpoint schema version");
    assert_eq!(
        maximum, 102,
        "future lease changes have their own upgrade rehearsal"
    );
    let applied: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM _sqlx_migrations WHERE version IN (101, 102) ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("private-volume migration versions");
    assert_eq!(applied, vec![101, 102]);
    assert_eq!(
        migration_checksums(&pool).await,
        checksums,
        "historical migration checksums remain unchanged"
    );
    assert_eq!(
        snapshot(&pool, &fixture, attempt).await,
        before,
        "legacy identity, metadata, lease, and mailbox evidence survive in place"
    );
    assert_eq!(
        execution_indexes(&pool).await,
        indexes,
        "exclusive lease indexes and mailbox constraints survive upgrade"
    );
    verify_new_ownership_and_bindings(&pool, &fixture).await;
    verify_preserved_fencing(&pool, &fixture, attempt).await;
    verify_standalone_volume(&pool, &fixture).await;
    println!(
        "REAL_PRIVATE_VOLUME_UPGRADE=1 historical_schema=100 checkpoint_schema=102 applied=101,102 legacy_rows_preserved=1 mailbox_lease_fence=13 old_checksums_preserved=1 standalone_volume=1 backing_reference_preserved=1 database_only=1"
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

async fn seed_execution_evidence(pool: &PgPool, fixture: &support::Fixture) -> Uuid {
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2, created_by = $3 WHERE id = $1")
        .bind(fixture.instance)
        .bind(fixture.volume)
        .bind(fixture.maintainer.as_uuid())
        .execute(pool)
        .await
        .expect("legacy instance pointer and creation provenance");
    sqlx::query("UPDATE agent_instance_state_volumes SET state = 'attached', lease_generation = $2,
        key_reference = 'legacy-key-reference', encryption_version = 2, backup_revision = 7,
        checksum = 'legacy-durable-backup-checksum', last_successful_backup_at = '2026-09-01T00:00:00Z' WHERE id = $1")
        .bind(fixture.volume).bind(FENCE).execute(pool).await.expect("nondefault legacy volume evidence");
    sqlx::query("UPDATE runs SET lease_fencing_token = $2, run_kind = 'normal', attachment_id = $3 WHERE id = $1")
        .bind(fixture.run)
        .bind(FENCE)
        .bind(fixture.attachment)
        .execute(pool)
        .await
        .expect("run fencing evidence");
    let lease = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_volume_leases
        (id, volume_id, instance_id, run_id, host_id, fencing_token, state, acquired_at,
         heartbeat_at, expires_at, attached_at)
        VALUES ($1, $2, $3, $4, 'host', $5, 'active', now(), now(), now() + interval '1 hour', now())")
        .bind(lease).bind(fixture.volume).bind(fixture.instance).bind(fixture.run).bind(FENCE)
        .execute(pool).await.expect("active exact legacy lease");
    let mailbox = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO mailboxes (id, project_id, instance_id, state) VALUES ($1, $2, $3, 'active')",
    )
    .bind(mailbox)
    .bind(fixture.consuming_project)
    .bind(fixture.instance)
    .execute(pool)
    .await
    .expect("legacy mailbox");
    let body = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO mailbox_payloads (id, mailbox_id, project_id, encoded_body,
        encoded_length, decoded_length, integrity_hash) VALUES ($1, $2, $3, $4, 4, 4, $5)",
    )
    .bind(body)
    .bind(mailbox)
    .bind(fixture.consuming_project)
    .bind(b"work".as_slice())
    // Exact SHA-256 of the four immutable accepted payload bytes above.
    .bind(
        [
            0_u8, 225, 62, 215, 175, 85, 178, 118, 34, 241, 214, 234, 181, 190, 192, 20, 126, 104,
            239, 226, 141, 194, 177, 36, 97, 17, 122, 250, 26, 94, 212, 14,
        ]
        .as_slice(),
    )
    .execute(pool)
    .await
    .expect("immutable accepted body");
    let event = Uuid::new_v4();
    sqlx::query("INSERT INTO mailbox_events (id, mailbox_id, project_id, instance_id, body_id,
        producer_kind, producer_id, deduplication_scope, deduplication_key, method, route, received_at)
        VALUES ($1, $2, $3, $4, $5, 'user', $6, 'upgrade', 'accepted-before-upgrade', 'POST', '/work', now())")
        .bind(event).bind(mailbox).bind(fixture.consuming_project).bind(fixture.instance).bind(body)
        .bind(fixture.maintainer.to_string()).execute(pool).await.expect("accepted historical event");
    sqlx::query("UPDATE mailbox_deliveries SET disposition = 'leased', logical_attempt_count = 1, dispatch_sequence = 1 WHERE event_id = $1")
        .bind(event).execute(pool).await.expect("claimed historical delivery");
    let attempt = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO mailbox_delivery_attempts (id, event_id, mailbox_id, attempt_number,
        state, command_id, instance_id, instance_revision_id, run_id, state_volume_id, lease_id,
        lease_fencing_token, state_access_outcome)
        SELECT $1, $2, $3, 1, 'leased', run.command_id, run.instance_id, run.instance_revision_id,
               run.id, $5, $6, $7, 'uncertain_access' FROM runs run WHERE run.id = $4",
    )
    .bind(attempt)
    .bind(event)
    .bind(mailbox)
    .bind(fixture.run)
    .bind(fixture.volume)
    .bind(lease)
    .bind(FENCE)
    .execute(pool)
    .await
    .expect("valid exact pre-provisioning volume/lease/fence triple");
    attempt
}

async fn snapshot(pool: &PgPool, fixture: &support::Fixture, attempt: Uuid) -> Value {
    let volume: Value = sqlx::query_scalar("SELECT to_jsonb(volume) - 'project_id' FROM agent_instance_state_volumes volume WHERE id = $1")
        .bind(fixture.volume).fetch_one(pool).await.expect("legacy volume snapshot");
    let lease: Value = sqlx::query_scalar("SELECT to_jsonb(lease) FROM agent_instance_volume_leases lease WHERE volume_id = $1 AND released_at IS NULL")
        .bind(fixture.volume).fetch_one(pool).await.expect("active lease snapshot");
    let instance: Value =
        sqlx::query_scalar("SELECT to_jsonb(instance) FROM agent_instances instance WHERE id = $1")
            .bind(fixture.instance)
            .fetch_one(pool)
            .await
            .expect("instance pointer/provenance snapshot");
    let run: Value = sqlx::query_scalar("SELECT to_jsonb(run) FROM runs run WHERE id = $1")
        .bind(fixture.run)
        .fetch_one(pool)
        .await
        .expect("run snapshot");
    let mailbox: Value = sqlx::query_scalar("SELECT jsonb_build_object('attempt', to_jsonb(attempt), 'delivery', to_jsonb(delivery),
        'event', to_jsonb(event), 'body', to_jsonb(body), 'mailbox', to_jsonb(mailbox))
        FROM mailbox_delivery_attempts attempt JOIN mailbox_deliveries delivery ON delivery.event_id = attempt.event_id
        JOIN mailbox_events event ON event.id = attempt.event_id JOIN mailbox_payloads body ON body.id = event.body_id
        JOIN mailboxes mailbox ON mailbox.id = attempt.mailbox_id WHERE attempt.id = $1")
        .bind(attempt).fetch_one(pool).await.expect("exact immutable mailbox evidence snapshot");
    json!({"volume": volume, "lease": lease, "instance": instance, "run": run, "mailbox": mailbox})
}

async fn migration_checksums(pool: &PgPool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as(
        "SELECT version, checksum FROM _sqlx_migrations WHERE version <= 100 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("historical applied migration checksums")
}

async fn execution_indexes(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
        'indexes', (SELECT jsonb_agg(jsonb_build_object('name', indexname, 'definition', indexdef) ORDER BY indexname)
            FROM pg_indexes WHERE schemaname = 'public' AND tablename IN ('agent_instance_volume_leases', 'mailbox_delivery_attempts')),
        'constraints', (SELECT jsonb_agg(jsonb_build_object('name', conname, 'definition', pg_get_constraintdef(oid)) ORDER BY conname)
            FROM pg_constraint WHERE conrelid IN ('agent_instance_volume_leases'::regclass, 'mailbox_delivery_attempts'::regclass)))")
        .fetch_one(pool).await.expect("execution indexes and constraints")
}

async fn verify_new_ownership_and_bindings(pool: &PgPool, fixture: &support::Fixture) {
    let project: Uuid =
        sqlx::query_scalar("SELECT project_id FROM agent_instance_state_volumes WHERE id = $1")
            .bind(fixture.volume)
            .fetch_one(pool)
            .await
            .expect("backfilled project owner");
    assert_eq!(project, fixture.consuming_project);
    let bindings: Vec<(Uuid, Uuid, Uuid, String, String, String)> = sqlx::query_as(
        "SELECT instance_id, project_id, volume_id, slot_key, guest_path, provenance
         FROM agent_instance_revision_volume_bindings WHERE instance_id = $1 ORDER BY instance_revision_id",
    ).bind(fixture.instance).fetch_all(pool).await.expect("backfilled immutable revision bindings");
    assert_eq!(
        bindings.len(),
        2,
        "both historical revisions retain exact state-volume scope"
    );
    for (instance, owner, volume, slot, path, provenance) in bindings {
        assert_eq!(
            (instance, owner, volume),
            (fixture.instance, fixture.consuming_project, fixture.volume)
        );
        assert_eq!(
            (slot.as_str(), path.as_str(), provenance.as_str()),
            ("state", "/var/lib/hephaestus", "legacy_state")
        );
    }
    let forced_rls: bool = sqlx::query_scalar("SELECT relrowsecurity AND relforcerowsecurity FROM pg_class WHERE oid = 'agent_instance_state_volumes'::regclass")
        .fetch_one(pool).await.expect("volume RLS flags");
    assert!(forced_rls);
}

async fn verify_preserved_fencing(pool: &PgPool, fixture: &support::Fixture, attempt: Uuid) {
    let error =
        sqlx::query("UPDATE mailbox_delivery_attempts SET lease_fencing_token = $2 WHERE id = $1")
            .bind(attempt)
            .bind(FENCE + 1)
            .execute(pool)
            .await
            .expect_err("stale mailbox fence remains rejected");
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    let error = sqlx::query(
        "INSERT INTO agent_instance_volume_leases
        (id, volume_id, instance_id, run_id, host_id, fencing_token, state, expires_at)
        VALUES ($1, $2, $3, $4, 'host', $5, 'active', now() + interval '1 hour')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.volume)
    .bind(fixture.instance)
    .bind(fixture.run)
    .bind(FENCE + 1)
    .execute(pool)
    .await
    .expect_err("exclusive active lease still rejects reuse");
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23505")
    );
}

async fn verify_standalone_volume(pool: &PgPool, fixture: &support::Fixture) {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes (id, project_id, state, capacity_bytes)
        VALUES ($1, $2, 'uninitialized', 16777216)",
    )
    .bind(id)
    .bind(fixture.consuming_project)
    .execute(pool)
    .await
    .expect("new standalone project-owned volume");
    let row: (Uuid, Option<Uuid>, i64) = sqlx::query_as("SELECT project_id, instance_id, capacity_bytes FROM agent_instance_state_volumes WHERE id = $1")
        .bind(id).fetch_one(pool).await.expect("standalone resource projection");
    assert_eq!(row, (fixture.consuming_project, None, 16_777_216));
}
