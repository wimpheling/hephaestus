//! Actor-role typed import and permanent admission proof on a populated upgrade.

use authz_postgres::PostgresMelangeAuthorizer;
use release_domain::{AgentInstanceId, InstanceRemovalId, ReleaseCommandKey};
use release_postgres::{ReleaseService, ReleaseServiceError};
use release_service::RequestInstanceRemoval;
use sqlx::{PgPool, migrate::Migrator};
use std::{borrow::Cow, sync::Arc};
use uuid::Uuid;

#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
// These shared graph helpers intentionally include unrelated matrix fields.
#[path = "volume_instances/fixture.rs"]
mod fixture;
#[path = "volume_instances/imports.rs"]
mod imports;
#[path = "volume_instances/mailbox.rs"]
mod mailbox;
#[path = "volume_instances/race.rs"]
mod race;
#[path = "volume_instances/removal.rs"]
mod removal;
#[allow(dead_code)]
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
mod support;
#[path = "volume_instances/unsupported.rs"]
mod unsupported;
// Reuse the already verified actor/worker and fenced legacy fixtures.
#[allow(dead_code)]
#[path = "volume_mount_grants/parity.rs"]
mod history;
#[allow(dead_code)]
#[path = "volume_mount_grants/fixture.rs"]
mod roles;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn typed_import_and_permanent_removal_admission_are_atomic(pool: PgPool) {
    checkpoint(106)
        .run(&pool)
        .await
        .expect("previous schema106");
    let mut seeded = seed::seed(&pool).await;
    let legacy = roles::legacy(&pool, &seeded).await;
    seeded.run = legacy.1.as_uuid();
    let lease = history::seed_active_lease(&pool, &seeded).await;
    let mailbox = mailbox::seed(&pool, &seeded, lease).await;
    let old_mailbox = mailbox::attempt_snapshot(&pool, mailbox).await;
    let before = upgrade_snapshot(&pool, &seeded, lease).await;
    checkpoint(107)
        .run(&pool)
        .await
        .expect("upgrade populated106 to107");
    assert_eq!(upgrade_snapshot(&pool, &seeded, lease).await, before);
    assert_eq!(mailbox::attempt_snapshot(&pool, mailbox).await, old_mailbox);
    let mode: String = sqlx::query_scalar("SELECT volume_mode FROM agent_instances WHERE id=$1")
        .bind(seeded.instance)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(mode, "legacy");
    race::progress_contention_fails_fast(&pool, &seeded).await;
    let actors = roles::role_pool(&pool, roles::Role::Actor).await;
    let service = ReleaseService::new(actors.clone(), Arc::new(PostgresMelangeAuthorizer));
    let named = imports::exercise(&pool, &seeded, &service).await;
    removal::named(&pool, &seeded, &service, &named).await;
    removal::exercise(&pool, &seeded, &service, lease).await;
    mailbox::assert_closed(&pool, &seeded, mailbox, &old_mailbox).await;
    actors.close().await;
    println!(
        "REAL_TYPED_INSTANCE_IMPORT_REMOVAL=1 upgrade106_to107=1 old_hash_checksums_bindings_active_lease_fences_preserved=1 actor_role=1 exact_pins_slots_modes_capacity=1 authorization_before_replay=1 atomic_grants_no_scalar_fallback=1 named_dispatch_closed=1 permanent_close=1 durable_cancellation=1 no_cleanup_claim=1"
    );
}

async fn upgrade_snapshot(
    pool: &PgPool,
    fixture: &support::Fixture,
    lease: Uuid,
) -> serde_json::Value {
    let checksum: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=106 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let hash: Vec<u8> =
        sqlx::query_scalar("SELECT runtime_contract_hash FROM release_agents WHERE id=$1")
            .bind(fixture.release_agent)
            .fetch_one(pool)
            .await
            .unwrap();
    serde_json::json!({"checksums":checksum,"hash":hash,"bindings":history::binding_snapshot(pool).await,"volume":history::volume_snapshot(pool,fixture.volume).await,"lease":history::lease_snapshot(pool,lease).await,"run":history::run_evidence(pool,fixture.run).await,"permission":history::legacy_read_permission(pool,fixture).await})
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
        ..MIGRATIONS
    }
}

fn removal_command(instance: Uuid, version: u64) -> RequestInstanceRemoval {
    RequestInstanceRemoval {
        command_key: ReleaseCommandKey::derive("remove-instance", &[Uuid::new_v4().as_bytes()]),
        removal_id: InstanceRemovalId::new(),
        instance_id: AgentInstanceId::from_uuid(instance),
        expected_version: version,
    }
}
