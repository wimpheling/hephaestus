use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AttemptProvenance, DeploymentError, DeploymentLifecycle, InstallProgress, ReconciledOutcome,
    ResourceAction,
};
use recipe_postgres::VerifiedOutcome;
use sqlx::PgPool;

use crate::{
    fixtures,
    harness::{self, Harness},
    support,
};

async fn replacement_manager(pool: &PgPool, h: &Harness) -> AuthenticatedIdentity {
    sqlx::query("INSERT INTO project_maintainers (project_id,user_id) VALUES ($1,$2)")
        .bind(h.fixture.consuming_project)
        .bind(h.fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .expect("new target manager");
    sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
        .bind(h.fixture.maintainer.as_uuid())
        .execute(pool)
        .await
        .expect("withdraw original actor");
    sqlx::query("UPDATE releases SET state='revoked', revoked_at=now() WHERE id=$1")
        .bind(h.fixture.release)
        .execute(pool)
        .await
        .expect("withdraw source");
    support::identity(h.fixture.outsider)
}

async fn reject_borrowed_commands(
    h: &Harness,
    manager: &AuthenticatedIdentity,
    original_actor: &AuthenticatedIdentity,
    target: &recipe_application::EffectClaim,
    observation: AttemptProvenance,
) {
    assert!(matches!(
        h.repository
            .record_reconciliation(
                manager,
                h.command,
                target,
                observation,
                ReconciledOutcome::NotApplied
            )
            .await,
        Err(DeploymentError::InputConflict)
    ));
    assert!(matches!(
        h.repository
            .complete_resource_effect(original_actor, target, harness::evidence(target))
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
}

async fn recover(pool: &PgPool, applied: bool) {
    let h = Harness::new(pool, true, false).await;
    let original_actor = h.request();
    let request = h
        .effect(&original_actor, h.command, "data", ResourceAction::Create)
        .await;
    let target = h
        .repository
        .claim_resource_effect(&original_actor, request)
        .await
        .expect("original claim");
    h.repository
        .mark_resource_ambiguous(
            &original_actor,
            &target,
            recipe_application::DiagnosticCode::ProviderOutcomeUnknown,
        )
        .await
        .expect("ambiguous creation");
    let manager = replacement_manager(pool, &h).await;
    let observation =
        AttemptProvenance::new(&manager, harness::attempt()).expect("new actor observation");
    reject_borrowed_commands(&h, &manager, &original_actor, &target, observation).await;
    let snapshot = h
        .repository
        .inspect(&manager, h.intent.id())
        .await
        .expect("new authorized inspection");
    let remove = fixtures::remove(&manager, h.intent.id(), snapshot.version);
    h.repository
        .admit_remove(&manager, remove)
        .await
        .expect("new manager cleanup command");
    let (verified, outcome) = if applied {
        (
            VerifiedOutcome::Applied,
            ReconciledOutcome::Applied(harness::evidence(&target)),
        )
    } else {
        (VerifiedOutcome::NotApplied, ReconciledOutcome::NotApplied)
    };
    h.proof(&target, observation.attempt_id, verified).await;
    let recovered = h
        .repository
        .record_reconciliation(&manager, remove.command, &target, observation, outcome)
        .await
        .expect("current cleanup reconciles immutable old claim");
    assert_eq!(recovered.active_attempt, None);
    assert_eq!(
        recovered.install,
        if applied {
            InstallProgress::Ready
        } else {
            InstallProgress::Failed
        }
    );
    let journal: (uuid::Uuid, uuid::Uuid, uuid::Uuid) = sqlx::query_as("SELECT command_id,attempt_id,actor_id FROM recipe_execution_transitions WHERE operation_id=$1 AND kind='reconciled'")
        .bind(observation.attempt_id.as_uuid()).fetch_one(pool).await.expect("current command and immutable target provenance");
    assert_eq!(
        journal,
        (
            remove.command.id().as_uuid(),
            target.provenance.attempt_id.as_uuid(),
            manager.user_id.as_uuid()
        )
    );
    let actor: uuid::Uuid =
        sqlx::query_scalar("SELECT actor_id FROM recipe_effect_attempts WHERE id=$1")
            .bind(target.provenance.attempt_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("original immutable actor");
    assert_eq!(actor, original_actor.user_id.as_uuid());
    let mut revived = h
        .effect(&manager, remove.command, "data", ResourceAction::Create)
        .await;
    revived.command = h.command;
    assert!(matches!(
        h.repository.claim_resource_effect(&manager, revived).await,
        Err(DeploymentError::InputConflict)
    ));
    finish_cleanup(pool, &h, &manager, remove.command).await;
}

async fn finish_cleanup(
    pool: &PgPool,
    h: &Harness,
    manager: &AuthenticatedIdentity,
    command: recipe_application::CommandIdentity,
) {
    for action in [
        ResourceAction::Drain,
        ResourceAction::Detach,
        ResourceAction::Delete,
    ] {
        h.apply(manager, command, "sqlite", action).await;
    }
    for action in [ResourceAction::Detach, ResourceAction::Retain] {
        h.apply(manager, command, "data", action).await;
    }
    let ready = h
        .repository
        .inspect(manager, h.intent.id())
        .await
        .expect("safe cleanup terminal");
    let finished = h
        .repository
        .finish_command(manager, command, h.intent.id(), ready.version)
        .await
        .expect("current manager terminal receipt");
    assert_eq!(finished.lifecycle, DeploymentLifecycle::Removed);
    let source_checks: i64 = sqlx::query_scalar("SELECT count(*) FROM authorization_audit_events WHERE actor_id=$1 AND request_id=$2 AND permission='can_use'")
        .bind(manager.user_id.as_uuid()).bind(manager.request_id.as_uuid()).fetch_one(pool).await.expect("cleanup excludes source");
    assert_eq!(source_checks, 0);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn different_manager_reconciles_applied_creation_after_actor_and_source_withdrawal(
    pool: PgPool,
) {
    recover(&pool, true).await;
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn different_manager_reconciles_verified_absence_after_actor_and_source_withdrawal(
    pool: PgPool,
) {
    recover(&pool, false).await;
}
