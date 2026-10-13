use super::fixture::Fixture;
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, EffectEvidence, EffectOutcome,
    PreparedInstanceSafetyRepository, PreparedInstanceSafetyState, ReconciledOutcome,
};
use recipe_postgres::{PostgresEffectVerificationRecorder, VerifiedOutcome};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL; no VM execution"]
async fn actual_commit_gap_and_source_loss_reconcile_under_real_current_remove(pool: PgPool) {
    let f = Fixture::new(&pool, true, false).await;
    let manager = f.manager().await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(f.base.fixture.release)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
        .bind(f.base.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut access = authz_postgres::begin_actor_transaction(&f.base.app, &manager)
        .await
        .unwrap();
    let old_source_use: i32 =
        sqlx::query_scalar("SELECT check_permission('user',$1,'can_use','release_agent',$2)")
            .bind(manager.user_id.to_string())
            .bind(f.base.fixture.release_agent.to_string())
            .fetch_one(&mut *access)
            .await
            .unwrap();
    access.rollback().await.unwrap();
    assert_eq!(
        old_source_use, 0,
        "current Remove borrows no installer Source authority"
    );
    let command = f.admit_remove(&manager).await;
    let before = f.snapshot().await;
    let inspected = f
        .service()
        .inspect_for_remove(&manager, command, &f.target)
        .await
        .unwrap();
    assert_eq!(inspected.state, PreparedInstanceSafetyState::Created);
    assert_eq!(
        f.service().observe_safety(&f.target).await.unwrap(),
        inspected
    );
    assert_eq!(
        f.snapshot().await,
        before,
        "readonly observation performs no import or outcome mutation"
    );
    // Actual worker observation above qualifies genuine committed107 creation.
    // This existing recorder writes creation evidence, not a fabricated VM receipt.
    let provenance = AttemptProvenance::new(
        &manager,
        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
    )
    .unwrap();
    PostgresEffectVerificationRecorder::new(f.base.worker.clone())
        .record(
            f.target.original(),
            provenance.attempt_id,
            VerifiedOutcome::Applied,
        )
        .await
        .unwrap();
    let progress = f
        .base
        .recipe
        .record_reconciliation(
            &manager,
            command,
            f.target.original(),
            provenance,
            ReconciledOutcome::Applied(EffectEvidence {
                identity: f.target.original().identity,
                input_hash: f.target.original().input_hash,
                outcome: EffectOutcome::Applied,
            }),
        )
        .await
        .unwrap();
    assert!(progress.active_attempt.is_none());
    f.close(&manager).await;
    let before = f.snapshot().await;
    let observed = f.service().observe_safety(&f.target).await.unwrap();
    assert_eq!(observed.state, PreparedInstanceSafetyState::Drained);
    assert!(observed.inventory.runs.is_empty() && observed.inventory.leases.is_empty());
    assert!(observed.removal_id.is_some());
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn unimported_original_claim_never_becomes_absence_or_ready(pool: PgPool) {
    let f = Fixture::new(&pool, false, false).await;
    let manager = f.manager().await;
    let command = f.admit_remove(&manager).await;
    let before = f.snapshot().await;
    let result = f
        .service()
        .inspect_for_remove(&manager, command, &f.target)
        .await
        .unwrap();
    assert_eq!(result.state, PreparedInstanceSafetyState::Unproven);
    assert!(result.removal_id.is_none());
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn closure_without_original_completion_stays_held(pool: PgPool) {
    let f = Fixture::new(&pool, true, false).await;
    let manager = f.manager().await;
    let command = f.admit_remove(&manager).await;
    f.close(&manager).await;
    let before = f.snapshot().await;
    let result = f
        .service()
        .inspect_for_remove(&manager, command, &f.target)
        .await
        .unwrap();
    assert_eq!(result.state, PreparedInstanceSafetyState::ClosedHeld);
    assert!(result.removal_id.is_some());
    assert!(result.inventory.runs.is_empty());
    assert_eq!(f.snapshot().await, before);
}
