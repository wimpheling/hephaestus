use run_domain::{RunOutcome, RunState};
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use vm_trait::VmExit;
use volume_trait::VolumeMetadataRepository;

use super::fixture::{self, Fixture};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn legacy_exact_acquire_bind_release_and_terminal_callback(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, true).await;
    let lease = f.acquire().await.unwrap();
    let prebound = f.runs.get(f.command.run_id).await.unwrap();
    assert_eq!(prebound.lease_id, Some(lease.id));
    assert_eq!(prebound.lease_fencing_token, Some(lease.fencing_token));
    assert!(prebound.vm_id.is_none());
    f.to_running(Some(&lease)).await;
    let exit = VmExit {
        code: Some(0),
        signal: None,
    };
    f.runs
        .transition(f.command.run_id, RunState::Succeeded, Some(&exit), None)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    let closed: bool =
        sqlx::query_scalar("SELECT volume_acquisition_closed_at IS NOT NULL FROM runs WHERE id=$1")
            .bind(f.command.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(closed);
    assert!(
        f.runs
            .transition(f.command.run_id, RunState::CleanedUp, None, None)
            .await
            .is_err()
    );
    // Controlled trusted provider boundary; this is not physical VM proof.
    f.volumes.release_after_detach(&lease, false).await.unwrap();
    let cleaned = f
        .runs
        .transition(f.command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    assert_eq!(cleaned.outcome, Some(RunOutcome::Succeeded));
    assert_no_receipt(&pool, f.command.run_id).await;
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn stateless_actual_bound_callback_and_positive_pre_io_failure_are_distinct(pool: PgPool) {
    fixture::schema(&pool).await;
    let bound = Fixture::new(&pool, false).await;
    assert!(bound.acquire().await.is_none());
    bound.to_running(None).await;
    bound
        .runs
        .transition(
            bound.command.run_id,
            RunState::Succeeded,
            Some(&VmExit {
                code: Some(0),
                signal: None,
            }),
            None,
        )
        .await
        .unwrap();
    finish_empty(&bound).await;
    assert!(
        bound
            .runs
            .get(bound.command.run_id)
            .await
            .unwrap()
            .vm_id
            .is_some()
    );
    let pre_io = Fixture::new(&pool, true).await;
    pre_io.runs.create_run(&pre_io.command).await.unwrap();
    pre_io
        .runs
        .transition(
            pre_io.command.run_id,
            RunState::Failed,
            None,
            Some("controlled pre-IO failure"),
        )
        .await
        .unwrap();
    finish_empty(&pre_io).await;
    assert!(
        pre_io
            .runs
            .get(pre_io.command.run_id)
            .await
            .unwrap()
            .vm_id
            .is_none()
    );
    assert_no_receipt(&pool, bound.command.run_id).await;
    assert_no_receipt(&pool, pre_io.command.run_id).await;
    bound.worker.close().await;
    pre_io.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn released_before_terminal_crash_replay_never_resets_new_generation(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, true).await;
    let old = f.acquire().await.unwrap();
    f.to_running(Some(&old)).await;
    f.runs
        .transition(
            f.command.run_id,
            RunState::Failed,
            None,
            Some("controlled exit"),
        )
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    f.volumes.release_after_detach(&old, false).await.unwrap();
    let next = super::support::retry_command(&f.command);
    let (_, repository) = super::support::seed_run_request_scope(&pool, &f.command).await;
    super::support::seed_retry_request(&pool, &next, repository, f.command.run_id).await;
    f.runs.create_run(&next).await.unwrap();
    f.runs
        .transition(next.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let fresh = f
        .volumes
        .acquire(
            old.volume_id,
            next.run_id,
            "lifecycle-host",
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert!(fresh.fencing_token > old.fencing_token);
    let before = fixture::evidence(&pool, next.run_id).await;
    f.runs
        .transition(f.command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    f.volumes.release_after_detach(&old, false).await.unwrap();
    assert_eq!(fixture::evidence(&pool, next.run_id).await, before);
    let replay = f.runs.create_run(&f.command).await.unwrap();
    assert!(!replay.created);
    assert_eq!(replay.run.state, RunState::CleanedUp);
    assert_eq!(fixture::evidence(&pool, next.run_id).await, before);
    f.worker.close().await;
}

async fn finish_empty(f: &Fixture) {
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    assert_eq!(
        f.runs
            .transition(f.command.run_id, RunState::CleanedUp, None, None)
            .await
            .unwrap()
            .state,
        RunState::CleanedUp
    );
}

async fn assert_no_receipt(pool: &PgPool, run: runtime_types::RunId) {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1")
            .bind(run.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        count, 0,
        "scalar cleanup must not fabricate canonical physical evidence"
    );
}
