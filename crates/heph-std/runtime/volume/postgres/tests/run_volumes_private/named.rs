use sqlx::PgPool;
use uuid::Uuid;
use volume_domain::{RunVolumeSelections, VolumeSelectionOrigin};
use volume_trait::{RunVolumeMetadataRepository, VolumeError, VolumeMetadataRepository};

use crate::{fixture, support::Fixture};

#[path = "cleanup.rs"]
mod cleanup;
#[path = "locks.rs"]
mod locks;
#[path = "named_fixture.rs"]
mod named_fixture;

pub async fn exercise(pool: &PgPool, seeded: &Fixture) {
    fixture::apply_private(pool).await;
    let (instance, revision, agent) = named_fixture::consumer(pool, seeded, 32).await;
    let run = unsupported_profile(pool, seeded, instance, revision, agent).await;
    let worker = fixture::worker(pool).await;
    let set = worker
        .load_run_selections(run)
        .await
        .expect("authoritative32-slot graph with actual live grants");
    assert_eq!(set.selections().len(), 32);
    assert!(
        set.selections()
            .iter()
            .all(|item| item.origin() == VolumeSelectionOrigin::Explicit)
    );
    let (now, expiry) = fixture::times();
    assert!(matches!(
        worker.acquire_run(&set, "wrong", now, expiry).await,
        Err(VolumeError::WrongHost { .. })
    ));
    assert_eq!(fixture::count(pool, run).await, 0);
    let leases = worker
        .acquire_run(&set, "host", now, expiry)
        .await
        .expect("atomic32 exclusive leases");
    assert_eq!(leases.len(), 32);
    deny_scalar_projection(&worker, run, &leases).await;
    let mut reversed = set.selections().to_vec();
    reversed.reverse();
    let reordered =
        RunVolumeSelections::new(set.identity(), reversed).expect("bounded canonical reorder");
    assert_eq!(
        worker
            .acquire_run(&reordered, "host", now, expiry)
            .await
            .expect("same set retry"),
        leases
    );
    worker
        .mark_run_attached(&set, "host", now, expiry)
        .await
        .expect("complete attached set");
    issuer_withdrawal(pool, seeded, &set).await;
    empty_set(pool, seeded).await;
    closed_acquisition(pool, seeded).await;
    cleanup::exercise(pool, seeded).await;
    locks::exercise(pool, seeded).await;
    println!(
        "PRIVATE_108_DB_ONLY=1 production_named_unsupported=1 supported_predicate_explicitly_simulated=1 exact32_ro_rw=1 immutable_replay=1 global_foreign_host_refusal=1 live_grant_issuer_loss_atomic_heartbeat=1 zero_set=1 close_before_acquire=1 swapped_slots_global_lock_order=1 no_vm_dispatch_claim=1"
    );
}

async fn deny_scalar_projection(
    worker: &volume_postgres::PostgresVolumeMetadataRepository,
    run: runtime_types::RunId,
    leases: &[volume_trait::RunVolumeLease],
) {
    let (now, expiry) = fixture::times();
    assert!(
        worker
            .acquire(leases[0].lease().volume_id, run, "host", now, expiry)
            .await
            .is_err()
    );
    assert!(worker.active_lease_for_run(run, "host").await.is_err());
    assert!(
        worker
            .mark_attached(leases[0].lease(), now, expiry)
            .await
            .is_err()
    );
}

async fn simulate_support(pool: &PgPool, supported: bool) {
    // Only this SQLx-owned disposable database changes the support predicate.
    if supported {
        sqlx::raw_sql("CREATE OR REPLACE FUNCTION named_volume_dispatch_supported(p_instance uuid) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT true $$").execute(pool).await.expect("disposable runtime-support simulation");
    } else {
        sqlx::raw_sql("CREATE OR REPLACE FUNCTION named_volume_dispatch_supported(p_instance uuid) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$ SELECT false $$").execute(pool).await.expect("restore unsupported fixture gate");
    }
}

async fn empty_set(pool: &PgPool, seeded: &Fixture) {
    let (instance, revision, agent) = named_fixture::consumer(pool, seeded, 0).await;
    let run = named_fixture::try_run(pool, seeded, instance, revision, agent)
        .await
        .expect("stateless exact named run");
    let worker = fixture::worker(pool).await;
    let empty = worker
        .load_run_selections(run)
        .await
        .expect("zero slots with exact identity");
    let (now, expiry) = fixture::times();
    assert!(
        worker
            .acquire_run(&empty, "host", now, expiry)
            .await
            .expect("durable zero-set acquisition")
            .is_empty()
    );
    assert!(
        worker
            .mark_run_attached(&empty, "host", now, expiry)
            .await
            .expect("zero attached set")
            .is_empty()
    );
    let role_pool = fixture::worker_pool(pool).await;
    sqlx::query("UPDATE runs SET vm_id='planned-empty',vm_provider_namespace='fixture-provider',vm_host_id='host',vm_binding_kind='planned' WHERE id=$1")
        .bind(run.as_uuid()).execute(&role_pool).await.expect("known host even for zero leases");
    assert!(matches!(
        worker.acquire_run(&empty, "wrong", now, expiry).await,
        Err(VolumeError::WrongHost { .. })
    ));
}

async fn closed_acquisition(pool: &PgPool, seeded: &Fixture) {
    let worker = fixture::worker(pool).await;
    let role_pool = fixture::worker_pool(pool).await;
    let (now, expiry) = fixture::times();
    let (instance, revision, agent) = named_fixture::consumer(pool, seeded, 2).await;
    let run = named_fixture::try_run(pool, seeded, instance, revision, agent)
        .await
        .expect("closure race run");
    let set = worker
        .load_run_selections(run)
        .await
        .expect("exact race set");
    let mut closing = role_pool.begin().await.expect("closure transaction");
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(instance)
        .fetch_one(&mut *closing)
        .await
        .expect("closure consumer lock");
    sqlx::query("UPDATE runs SET state='cleaning_up' WHERE id=$1")
        .bind(run.as_uuid())
        .execute(&mut *closing)
        .await
        .expect("acquisition closed before competing transaction");
    let acquiring = worker.acquire_run(&set, "host", now, expiry);
    tokio::pin!(acquiring);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut acquiring)
            .await
            .is_err(),
        "acquire serializes behind closure"
    );
    closing.commit().await.expect("durable closure");
    assert!(acquiring.await.is_err(), "closed run cannot gain any lease");
    assert_eq!(fixture::count(pool, run).await, 0);
}

async fn unsupported_profile(
    pool: &PgPool,
    seeded: &Fixture,
    instance: Uuid,
    revision: Uuid,
    agent: Uuid,
) -> runtime_types::RunId {
    let false_helper: bool = sqlx::query_scalar("SELECT named_volume_dispatch_supported($1)")
        .bind(instance)
        .fetch_one(pool)
        .await
        .expect("production dispatch capability");
    assert!(!false_helper);
    let rejected = named_fixture::try_run(pool, seeded, instance, revision, agent)
        .await
        .expect_err("production named run admission denied");
    assert_eq!(
        rejected
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    simulate_support(pool, true).await;
    let run = named_fixture::try_run(pool, seeded, instance, revision, agent)
        .await
        .expect("explicit disposable supported-runtime simulation");
    simulate_support(pool, false).await;
    let worker = fixture::worker(pool).await;
    assert!(
        worker.load_run_selections(run).await.is_err(),
        "catalog keeps named unsupported"
    );
    let denied=sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at,selection_origin) SELECT gen_random_uuid(),volume_id,$2,$1,'host',1,'active',now()+interval '1 hour','explicit' FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=$3 AND slot_key='slot00'")
        .bind(run.as_uuid()).bind(instance).bind(revision).execute(pool).await.expect_err("lease admission keeps named unsupported");
    assert_eq!(
        denied
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    assert_eq!(fixture::count(pool, run).await, 0);
    simulate_support(pool, true).await;
    run
}

async fn issuer_withdrawal(pool: &PgPool, seeded: &Fixture, set: &RunVolumeSelections) {
    let worker = fixture::worker(pool).await;
    let run = set.identity().run_id();
    let (now, expiry) = fixture::times();
    let before: Vec<(Uuid, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT id,expires_at FROM agent_instance_volume_leases WHERE run_id=$1 ORDER BY id",
    )
    .bind(run.as_uuid())
    .fetch_all(pool)
    .await
    .expect("frozen expiry evidence");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seeded.consuming_project)
        .bind(seeded.maintainer.as_uuid())
        .execute(pool)
        .await
        .expect("withdraw actual creator source authority");
    assert!(
        worker
            .heartbeat_run(set, "host", now, expiry + time::Duration::minutes(1))
            .await
            .is_err()
    );
    let after: Vec<(Uuid, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT id,expires_at FROM agent_instance_volume_leases WHERE run_id=$1 ORDER BY id",
    )
    .bind(run.as_uuid())
    .fetch_all(pool)
    .await
    .expect("no partial heartbeat extension");
    assert_eq!(before, after);
    assert_eq!(
        worker
            .leases_for_run(run, "host")
            .await
            .expect("inspection survives authority loss")
            .len(),
        32
    );
    assert!(matches!(
        worker.leases_for_run(run, "wrong").await,
        Err(VolumeError::WrongHost { .. })
    ));
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(seeded.consuming_project)
        .bind(seeded.maintainer.as_uuid())
        .execute(pool)
        .await
        .expect("restore actual source authority");
}
