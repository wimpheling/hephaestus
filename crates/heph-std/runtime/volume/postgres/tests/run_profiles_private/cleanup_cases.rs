use run_domain::{RunCleanupHostId, RunCleanupReceipt, RunCleanupVmObservation};
use run_orchestrator::RunCleanupRepository;
use run_postgres::PgRunCleanupRepository;
use sqlx::{PgPool, postgres::PgPoolOptions};
use time::OffsetDateTime;
use vm_trait::VmId;

use super::super::{
    fixture::{self, Execution, Mutation},
    producers,
};

async fn ready(pool: &PgPool, volume: uuid::Uuid) {
    sqlx::query("UPDATE agent_instance_state_volumes SET provisioning_state='ready' WHERE id=$1")
        .bind(volume)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn owned_whole_set_receipt_remains_required_before_release_and_cleanup(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    ready(&pool, f.volume).await;
    producers::normal(&pool, &e, "owned_canonical", true)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "leasing_volume").await;
    let lease = fixture::lease(&pool, &e, &f).await.unwrap();
    fixture::denied(&pool, e.run, Mutation::Release).await;
    let worker = PgPoolOptions::new()
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let scope = RunCleanupHostId::new("profile-provider".into(), "host".into()).unwrap();
    let repository = PgRunCleanupRepository::new(worker, scope.clone());
    let target = repository.begin_cleanup(e.run.into()).await.unwrap();
    assert_eq!(target.leases().len(), 1);
    assert_eq!(target.leases()[0].lease_id().as_uuid(), lease);
    fixture::advance(&pool, e.run, "cleaning_up").await;
    fixture::denied(&pool, e.run, Mutation::Clean).await;
    // Syntactic trusted-worker observation exercises SQL semantics only; no VM
    // destruction took place, and no physical lifecycle claim is made.
    let receipt = RunCleanupReceipt::new(
        target,
        RunCleanupVmObservation::authoritatively_absent(scope, VmId(e.run.to_string())).unwrap(),
        OffsetDateTime::now_utc(),
    )
    .unwrap();
    assert!(repository.finish_cleanup(&receipt).await.is_err());
    repository.record_cleanup_receipt(&receipt).await.unwrap();
    repository.finish_cleanup(&receipt).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL",
    )
    .bind(e.run)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn legacy_held_lease_release_is_deliberately_not_activated(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::normal(&pool, &f).await;
    ready(&pool, f.volume).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "leasing_volume").await;
    fixture::lease(&pool, &e, &f).await.unwrap();
    fixture::advance(&pool, e.run, "cleaning_up").await;
    fixture::denied(&pool, e.run, Mutation::Release).await;
    fixture::denied(&pool, e.run, Mutation::Clean).await;
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL",
    )
    .bind(e.run)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(held, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn legacy_empty_terminal_state_is_not_canonical_absence_evidence(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = Execution::stateless(&pool, &f).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, e.run, "cleaning_up").await;
    fixture::advance(&pool, e.run, "cleaned_up").await;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private profile PostgreSQL"]
async fn owned_update_exact_admission_blocks_io_until_single_worker_consumption(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let (e, result) = producers::update(&pool, &f, true).await;
    result.unwrap();
    fixture::denied(&pool, e.run, Mutation::Progress).await;
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now(),consumption_kind='provision',vm_id=run_id::text,vm_provider_namespace='profile-provider',vm_host_id='host' WHERE run_id=$1")
        .bind(e.run).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE runs SET vm_id=id::text,vm_provider_namespace='profile-provider',vm_host_id='host',vm_binding_kind='planned' WHERE id=$1")
        .bind(e.run).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    fixture::advance(&pool, e.run, "provisioning").await;
    let mut tx = fixture::worker(&pool).await;
    assert!(
        sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now() WHERE run_id=$1")
            .bind(e.run)
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
}
