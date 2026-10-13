//! Ordinary user Cancel before117: real producers and worker transactions only.
//! Stateless preparation proves no nonempty volume-grant or native VM acceptance.

use run_domain::{CancelRun, RunCleanupTarget, RunOutcome, RunState};
use run_orchestrator::RunRepository;
use runtime_types::CommandId;
use sqlx::PgPool;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

#[path = "queued_cancel/denial.rs"]
mod denial;
#[path = "queued_cancel/fixture.rs"]
mod fixture;
#[path = "queued_cancel/snapshot.rs"]
mod snapshot;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public140 PostgreSQL through DATABASE_URL and local Git"]
async fn ordinary_cancel_prepares_only_its_run_and_preserves_multiple_real_origins(pool: PgPool) {
    let f = fixture::Fixture::new(&pool).await;
    let neighbors = snapshot::neighbors(&pool, &f).await;
    assert_eq!(neighbors["instance"]["run_gate_open"], true);
    assert_eq!(neighbors["grants"], serde_json::json!([]));
    assert_eq!(neighbors["revocations"], serde_json::json!([]));
    let command = f.request(&pool, "ordinary user cancellation").await;
    let pending = snapshot::state(&pool, &f).await;
    let original = pending["original"].clone();
    let start_inbox = pending["inbox"][0].clone();
    assert!(
        f.runs
            .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
            .await
            .is_err()
    );
    assert_eq!(pending, snapshot::state(&pool, &f).await);
    let cancel = f.admit(&pool, &command).await;
    let emitted = snapshot::state(&pool, &f).await;
    assert!(emitted["run"]["cancel_requested_at"].is_null());
    assert!(
        f.runs
            .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
            .await
            .is_err(),
        "producer completion is not worker delivery"
    );
    assert_eq!(emitted, snapshot::state(&pool, &f).await);
    assert!(f.runs.request_cancel(&cancel).await.unwrap());
    let delivered = snapshot::state(&pool, &f).await;
    assert!(!f.runs.request_cancel(&cancel).await.unwrap());
    assert_eq!(delivered, snapshot::state(&pool, &f).await);
    let (run, target) = f
        .runs
        .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Queued);
    assert!(run.cancel_requested_at.is_some());
    assert_eq!(target.generation(), 1);
    assert!(target.leases().is_empty());
    assert_eq!(target.vm_target().host(), Some(&f.host));
    assert_eq!(
        target.vm_target().vm_id(),
        Some(&VmId(f.start.run_id.to_string()))
    );
    let prepared = snapshot::state(&pool, &f).await;
    let (_, replay) = f
        .runs
        .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
        .await
        .unwrap();
    assert_eq!(target, replay);
    assert_eq!(prepared, snapshot::state(&pool, &f).await);
    assert!(
        f.runs
            .create_run_with_vm_plan(&f.start, &f.host, &VmId(f.start.run_id.to_string()))
            .await
            .is_err()
    );
    assert_eq!(prepared, snapshot::state(&pool, &f).await);
    f.runs
        .transition(f.start.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
    assert_eq!(
        f.runs.get(f.start.run_id).await.unwrap().state,
        RunState::Cancelled
    );
    second_control_preserves_target(&pool, &f, command.command_id.as_uuid(), &target).await;
    let final_state = snapshot::state(&pool, &f).await;
    assert_eq!(original, final_state["original"]);
    assert!(
        final_state["inbox"]
            .as_array()
            .unwrap()
            .contains(&start_inbox)
    );
    assert_eq!(neighbors, snapshot::neighbors(&pool, &f).await);
    snapshot::unconsumed(&pool, &f).await;
    f.original.close().await;
}

async fn second_control_preserves_target(
    pool: &PgPool,
    f: &fixture::Fixture,
    first: Uuid,
    target: &RunCleanupTarget,
) {
    // Distinct requests can emit identical actor/reason event bodies. Both real
    // origins must remain valid without resetting the target or holding this Run.
    let second = f.request(pool, "ordinary user cancellation").await;
    assert_ne!(second.command_id.as_uuid(), first);
    let second_cancel = f.admit(pool, &second).await;
    assert!(f.runs.request_cancel(&second_cancel).await.unwrap());
    assert!(!f.runs.request_cancel(&second_cancel).await.unwrap());
    let (run, same) = f
        .runs
        .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
        .await
        .unwrap();
    assert_eq!(run.state, RunState::Cancelled);
    assert_eq!(&same, target);
    assert_eq!(run.outcome, Some(RunOutcome::Cancelled));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public140 PostgreSQL through DATABASE_URL and local Git"]
async fn exact_origin_owner_configuration_and_worker_are_required_without_effects(pool: PgPool) {
    let f = fixture::Fixture::new(&pool).await;
    denial::drain_observation_boundaries(&pool, &f).await;
    let neighbors = snapshot::neighbors(&pool, &f).await;
    let command = f.request(&pool, "real cancellation").await;
    let cancel = f.admit(&pool, &command).await;
    let original = snapshot::state(&pool, &f).await;
    let wrong_id = CancelRun {
        command_id: CommandId::new(),
        ..cancel.clone()
    };
    let wrong_reason = CancelRun {
        reason: "different DTO".into(),
        ..cancel.clone()
    };
    let wrong_run = CancelRun {
        run_id: f.other.run_id,
        ..cancel.clone()
    };
    for different in [wrong_id, wrong_reason, wrong_run] {
        assert!(f.runs.request_cancel(&different).await.is_err());
        assert_eq!(original, snapshot::state(&pool, &f).await);
    }
    let foreign =
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), f.host.host_id().into()).unwrap();
    assert!(
        f.runs
            .prepare_queued_cancel_cleanup(f.start.run_id, &foreign)
            .await
            .is_err()
    );
    let config = crate::fixture::configuration(&f.original.scope, "different/v2");
    let different = f.repository(f.original.worker.clone(), config);
    assert!(different.request_cancel(&cancel).await.is_err());
    let nonworker = f.repository(f.original.application.clone(), f.config.clone());
    assert!(nonworker.request_cancel(&cancel).await.is_err());
    assert_eq!(original, snapshot::state(&pool, &f).await);
    assert!(f.runs.request_cancel(&cancel).await.unwrap());
    let delivered = snapshot::state(&pool, &f).await;
    assert!(
        different
            .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
            .await
            .is_err()
    );
    assert!(
        nonworker
            .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
            .await
            .is_err()
    );
    assert!(
        f.runs
            .prepare_queued_cancel_cleanup(f.start.run_id, &foreign)
            .await
            .is_err()
    );
    assert_eq!(delivered, snapshot::state(&pool, &f).await);
    denial::pending_inbox_and_changed_origin(&pool, &f, &cancel).await;
    denial::held_original_reservation(&f, &cancel).await;
    assert_eq!(neighbors, snapshot::neighbors(&pool, &f).await);
    snapshot::unconsumed(&pool, &f).await;
    f.original.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public140 PostgreSQL through DATABASE_URL and local Git"]
async fn actual_consumption_cannot_be_reclassified_as_ordinary_zero_history(pool: PgPool) {
    let f = fixture::Fixture::new(&pool).await;
    let consumed = f
        .runs
        .create_run_with_vm_plan(&f.start, &f.host, &VmId(f.start.run_id.to_string()))
        .await
        .unwrap();
    assert!(
        !consumed.created,
        "the genuine Invoke already created this Run"
    );
    let command = f
        .request(&pool, "cancel consumed run through its existing boundary")
        .await;
    let cancel = f.admit(&pool, &command).await;
    assert!(f.runs.request_cancel(&cancel).await.unwrap());
    let original = snapshot::state(&pool, &f).await;
    assert!(
        f.runs
            .prepare_queued_cancel_cleanup(f.start.run_id, f.config.provider_scope())
            .await
            .is_err()
    );
    assert_eq!(original, snapshot::state(&pool, &f).await);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_invocation_consumptions WHERE run_id=$1")
            .bind(f.start.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert!(original["targets"].as_array().unwrap().is_empty());
    f.original.close().await;
}
