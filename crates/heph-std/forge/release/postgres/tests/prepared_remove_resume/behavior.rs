use super::fixture::Fixture;
use recipe_application::{
    ActiveDeploymentEffectRepository, AdmissionDisposition, AttemptProvenance, DeploymentAttemptId,
    DeploymentError, DiagnosticCode, ReconciledOutcome, RemoveResumeConfiguration,
};
use recipe_postgres::PostgresDeploymentRepository;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database; no VM or physical cleanup proof"]
async fn fresh_manager_preserves_original_closure_after_actor_and_source_withdrawal(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.withdraw_original().await;
    let before = f.preserved().await;
    let admitted = f
        .repository()
        .admit_remove_resume(&f.manager, f.remove)
        .await
        .unwrap();
    assert_eq!(
        admitted.admission().disposition,
        AdmissionDisposition::Created
    );
    assert_eq!(admitted.continuations().len(), 1);
    let old = &admitted.continuations()[0];
    assert_eq!(old.claim, *f.request.drain());
    assert_ne!(old.claim.command, f.remove.command);
    let closure = old.closure.as_ref().unwrap();
    assert_eq!(closure.removal_id, f.request.removal_id());
    assert_eq!(closure.command_key, f.request.command_key());
    assert_eq!(closure.drain, *f.request.drain());
    assert_eq!(f.preserved().await, before);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM recipe_remove_resume_admissions a JOIN recipe_deployment_commands c ON c.id=a.command_id
      JOIN application_events e ON e.id=a.event_id JOIN product_event_outbox o ON o.event_id=e.id
      WHERE c.id=$1 AND c.actor_id=$2 AND e.actor_id=c.actor_id AND e.request_id=c.first_request_id")
      .bind(f.remove.command.id().as_uuid()).bind(f.manager.user_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.repository()
            .admit_remove_resume(&f.manager, f.remove)
            .await
            .unwrap()
            .admission()
            .disposition,
        AdmissionDisposition::Resume
    );
    assert_eq!(f.preserved().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database; Unproven retains active work"]
async fn new_manager_loads_exact_old_claim_and_records_only_unproven_reconciliation(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.withdraw_original().await;
    f.repository()
        .admit_remove_resume(&f.manager, f.remove)
        .await
        .unwrap();
    let target = f
        .preparation()
        .load_active_effect(
            &f.manager,
            f.remove.command,
            f.remove.deployment_id,
            &f.request.drain().resource,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target, *f.request.drain());
    let observation = AttemptProvenance::new(
        &f.manager,
        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
    )
    .unwrap();
    let repository = f.repository();
    let outcome = repository
        .record_reconciliation(
            &f.manager,
            f.remove.command,
            &target,
            observation,
            ReconciledOutcome::Unproven(DiagnosticCode::ProviderFailure),
        )
        .await
        .unwrap();
    assert_eq!(outcome.active_attempt, Some(target.provenance.attempt_id));
    assert_eq!(
        repository
            .admit_remove_resume(&f.manager, f.remove)
            .await
            .unwrap()
            .admission()
            .disposition,
        AdmissionDisposition::Resume
    );
    let before = f.execution_snapshot().await;
    let replay = repository
        .record_reconciliation(
            &f.manager,
            f.remove.command,
            &target,
            observation,
            ReconciledOutcome::Unproven(DiagnosticCode::ProviderFailure),
        )
        .await
        .unwrap();
    assert_eq!(replay, outcome);
    assert_eq!(f.execution_snapshot().await, before);
    let mut changed = target.clone();
    changed.provenance.attempt_id = DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap();
    assert!(
        repository
            .record_reconciliation(
                &f.manager,
                f.remove.command,
                &changed,
                observation,
                ReconciledOutcome::Unproven(DiagnosticCode::ProviderFailure)
            )
            .await
            .is_err()
    );
    assert_eq!(f.execution_snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database"]
async fn default_unsupported_stale_cas_and_config_replay_deny_without_changes(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let before = f.snapshot().await;
    assert!(matches!(
        PostgresDeploymentRepository::new(f.closure.original.base.app.clone())
            .admit_remove_resume(&f.manager, f.remove)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    let mut stale = f.remove;
    stale.expected_version += 1;
    assert!(matches!(
        f.repository().admit_remove_resume(&f.manager, stale).await,
        Err(DeploymentError::StaleClaim)
    ));
    assert_eq!(f.snapshot().await, before);
    f.repository()
        .admit_remove_resume(&f.manager, f.remove)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let wrong = RemoveResumeConfiguration::new(
        "other-actual-owner".into(),
        f.config.host().into(),
        f.config.platform().clone(),
        f.config.profile(),
    )
    .unwrap();
    let other = PostgresDeploymentRepository::new(f.closure.original.base.app.clone())
        .with_remove_resume(wrong);
    assert!(matches!(
        other.admit_remove_resume(&f.manager, f.remove).await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database"]
async fn inactive_current_manager_and_worker_role_cannot_admit(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let active_before = f.snapshot().await;
    let worker = PostgresDeploymentRepository::new(f.closure.original.base.worker.clone())
        .with_remove_resume(f.config.clone());
    assert!(
        worker
            .admit_remove_resume(&f.manager, f.remove)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, active_before);
    let mut raw =
        authz_postgres::begin_actor_transaction(&f.closure.original.base.worker, &f.manager)
            .await
            .unwrap();
    let error = sqlx::query("SELECT reserve_remove_resume118($1)")
        .bind(f.remove.deployment_id.as_uuid())
        .execute(&mut *raw)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    raw.rollback().await.unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(f.manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert!(
        f.repository()
            .admit_remove_resume(&f.manager, f.remove)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 database"]
async fn management_withdrawal_denies_new_command_without_borrowing_old_actor(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.closure.original.base.fixture.consuming_project)
        .bind(f.manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.snapshot().await;
    assert!(
        f.repository()
            .admit_remove_resume(&f.manager, f.remove)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    let admitted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recipe_remove_resume_admissions WHERE command_id=$1",
    )
    .bind(f.remove.command.id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(admitted, 0);
}
