//! A real retry keeps per-Run scalar rows, never update-only legacy leases.

use super::{fixture, pending};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseCommandKey;
use release_service::{
    BeginUpdateHook, RecoverInstanceUpdate, UpdateDecision, UpdateHookResult, UpdateRecoveryAction,
    UpdateRecoveryDecision,
};
use run_domain::{LegacyVmPlacementScope, StartRun};
use runtime_types::RunId;
use sqlx::PgPool;

async fn first_attempt(
    pool: &PgPool,
    worker: &PgPool,
    scope: &LegacyVmPlacementScope,
) -> (
    AuthenticatedIdentity,
    BeginUpdateHook,
    StartRun,
    fixture::Lease,
) {
    let normal = fixture::fresh(pool, worker, scope, true, false).await;
    fixture::close(worker, scope, &normal).await;
    fixture::outcome(worker, &normal, false).await;
    fixture::transition(worker, &normal, "cleaned_up").await;
    let (actor, command) = pending(pool, &normal).await;
    fixture::service(worker, scope)
        .begin_update_hook(&actor, command.clone())
        .await
        .unwrap();
    let run = super::super::seed::load_hook(pool, command.hook_run_id).await;
    fixture::consume(worker, scope, &run).await;
    let lease = fixture::acquire(pool, worker, scope, &run, None).await;
    finish_attempt(worker, scope, &run, &lease).await;
    assert_eq!(
        fixture::service(worker, scope)
            .record_update_hook_result(command.update_id, UpdateHookResult::Uncertain)
            .await
            .unwrap(),
        UpdateDecision::CompatibilityUnknown
    );
    (actor, command, normal, lease)
}

async fn finish_attempt(
    worker: &PgPool,
    scope: &LegacyVmPlacementScope,
    run: &StartRun,
    lease: &fixture::Lease,
) {
    fixture::running(worker, run).await;
    fixture::close(worker, scope, run).await;
    fixture::outcome(worker, run, false).await;
    fixture::detach(worker, scope, run, lease).await;
    fixture::transition(worker, run, "cleaned_up").await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn retry_hook_requires_all_original_attempts_released_without_resetting_new_fence(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let (actor, first, normal, old_lease) = first_attempt(&pool, &worker, &scope).await;
    assert_eq!(
        fixture::service(&worker, &scope)
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: ReleaseCommandKey::derive(
                        "observer-retry",
                        &[first.update_id.as_uuid().as_bytes()]
                    ),
                    update_id: first.update_id,
                    action: UpdateRecoveryAction::RetryHook,
                }
            )
            .await
            .unwrap(),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    let second = BeginUpdateHook {
        command_key: ReleaseCommandKey::derive(
            "observer-retry-hook",
            &[first.update_id.as_uuid().as_bytes()],
        ),
        update_id: first.update_id,
        hook_run_id: RunId::new(),
    };
    fixture::service(&worker, &scope)
        .begin_update_hook(&actor, second.clone())
        .await
        .unwrap();
    let run = super::super::seed::load_hook(&pool, second.hook_run_id).await;
    fixture::consume(&worker, &scope, &run).await;
    let lease = fixture::acquire(&pool, &worker, &scope, &run, Some(old_lease.volume_id)).await;
    assert!(lease.fencing_token > old_lease.fencing_token);
    fixture::running(&worker, &run).await;
    fixture::close(&worker, &scope, &run).await;
    fixture::outcome(&worker, &run, false).await;
    let held = fixture::snapshot(&pool, &run).await;
    assert!(
        fixture::service(&worker, &scope)
            .record_update_hook_result(second.update_id, UpdateHookResult::Rejected(17))
            .await
            .is_err()
    );
    assert_eq!(fixture::snapshot(&pool, &run).await, held);
    fixture::detach(&worker, &scope, &run, &lease).await;
    fixture::transition(&worker, &run, "cleaned_up").await;
    assert_eq!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .unwrap(),
        UpdateDecision::AgentRejected
    );
    let qualified: bool = sqlx::query_scalar("SELECT count(*)=2 AND bool_and(lease.update_id IS NULL AND lease.state='released' AND lease.released_at IS NOT NULL) FROM run_legacy_vm_placements plan JOIN agent_instance_volume_leases lease ON lease.run_id=plan.run_id WHERE plan.producer_kind='update_hook' AND plan.producer_id=$1")
        .bind(first.update_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(qualified);
    let next = super::super::support::retry_command(&normal);
    fixture::birth(&pool, &worker, &scope, &next, false).await;
    let newer = fixture::acquire(&pool, &worker, &scope, &next, Some(lease.volume_id)).await;
    assert!(newer.fencing_token > lease.fencing_token);
    let before = fixture::snapshot(&pool, &run).await;
    fixture::service(&worker, &scope)
        .legacy_run_cleanup_completed(first.hook_run_id)
        .await
        .unwrap();
    fixture::service(&worker, &scope)
        .legacy_run_cleanup_completed(second.hook_run_id)
        .await
        .unwrap();
    assert_eq!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .unwrap(),
        UpdateDecision::AgentRejected
    );
    assert_eq!(fixture::snapshot(&pool, &run).await, before);
    worker.close().await;
}
