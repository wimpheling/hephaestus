use super::fixture::Closure;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, EffectEvidence, EffectOutcome,
    PreparedInstanceClosureRepository, PreparedInstanceSafetyRepository,
    PreparedInstanceSafetyState, ReconciledOutcome,
};
use recipe_postgres::{PostgresEffectVerificationRecorder, VerifiedOutcome};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL; no physical VM cleanup"]
async fn completed_original_loader_and_exact_closure_replay_preserve_terminal_data(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let before = f.snapshot().await;
    let loaded = f
        .original
        .service()
        .load_original_target(
            &f.manager,
            f.command,
            &f.original.base.intent,
            f.original.base.input.resource(),
        )
        .await
        .unwrap();
    assert_eq!(loaded, f.original.target);
    assert_eq!(f.snapshot().await, before);
    let request = f.request().await;
    let service = f.original.service();
    let admitted = service
        .close_for_remove(&f.manager, &request)
        .await
        .unwrap();
    assert_eq!(admitted.target, loaded);
    assert!(!admitted.replayed);
    let state: (bool, i64) =
        sqlx::query_as("SELECT run_gate_open,version FROM agent_instances WHERE id=$1")
            .bind(loaded.prepared().instance_id().as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!state.0);
    assert_eq!(
        u64::try_from(state.1).unwrap(),
        request.expected_instance_version() + 1
    );
    let before = f.snapshot().await;
    assert!(
        service
            .close_for_remove(&f.manager, &request)
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(
        f.snapshot().await,
        before,
        "replay has no gate/version, audit, event or cancellation mutation"
    );
    let observed = service.observe_safety(&loaded).await.unwrap();
    assert_eq!(observed.state, PreparedInstanceSafetyState::Drained);
    assert_eq!(observed.removal_id, Some(request.removal_id()));
    assert!(
        observed.inventory.runs.is_empty(),
        "database zero-Run boundary only"
    );
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn genuine_commit_gap_loads_original_and_reconciles_under_new_manager_without_source(
    pool: PgPool,
) {
    let f = Closure::new(&pool, false).await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(f.original.base.fixture.release)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
        .bind(f.original.base.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let loaded = f
        .original
        .service()
        .load_original_target(
            &f.manager,
            f.command,
            &f.original.base.intent,
            f.original.base.input.resource(),
        )
        .await
        .unwrap();
    assert_eq!(loaded, f.original.target);
    let provenance = AttemptProvenance::new(
        &f.manager,
        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
    )
    .unwrap();
    PostgresEffectVerificationRecorder::new(f.original.base.worker.clone())
        .record(
            loaded.original(),
            provenance.attempt_id,
            VerifiedOutcome::Applied,
        )
        .await
        .unwrap();
    f.original
        .base
        .recipe
        .record_reconciliation(
            &f.manager,
            f.command,
            loaded.original(),
            provenance,
            ReconciledOutcome::Applied(EffectEvidence {
                identity: loaded.original().identity,
                input_hash: loaded.original().input_hash,
                outcome: EffectOutcome::Applied,
            }),
        )
        .await
        .unwrap();
    let request = f.request().await;
    assert_eq!(
        request.target().original(),
        loaded.original(),
        "never rewrite the original Install claim"
    );
    f.original
        .service()
        .close_for_remove(&f.manager, &request)
        .await
        .unwrap();
    assert!(
        f.original
            .service()
            .observe_safety(&loaded)
            .await
            .unwrap()
            .removal_id
            .is_some()
    );
}
