//! Permanent Remove admission is stronger than a mutable lifecycle label.
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError,
    DeploymentExecutionPreparationRepository, DeploymentLifecycle, DiagnosticCode,
    ReconciledOutcome, ResourceAction, ResourceEffect,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    claim_fixture, fixtures,
    harness::{Graph, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn restored_creator_cannot_reopen_install_after_remove_and_recovery_required(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    let prepared = h.prepare().await;
    let manager = claim_fixture::manager(&h, &pool).await;
    let remove = claim_fixture::remove(&h, &manager).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(!h.can_use(&h.actor).await);
    h.repository
        .record_reconciliation(
            &manager,
            remove,
            &claim,
            AttemptProvenance::new(
                &manager,
                DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
            )
            .unwrap(),
            ReconciledOutcome::Unproven(DiagnosticCode::ProviderOutcomeUnknown),
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        h.can_use(&h.actor).await,
        "restored current SourceCanUse cannot reopen Install"
    );
    let snapshot = h.repository.inspect(&manager, h.intent.id()).await.unwrap();
    assert_eq!(snapshot.lifecycle, DeploymentLifecycle::RecoveryRequired);
    let before = claim_fixture::state(&pool).await;
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    assert!(matches!(
        claim_fixture::load(&h, &h.actor, h.command, &claim.resource).await,
        Err(DeploymentError::InvalidAction)
    ));
    assert_eq!(
        claim_fixture::load(&h, &manager, remove, &claim.resource)
            .await
            .unwrap(),
        Some(claim.clone())
    );
    assert!(matches!(
        h.repository
            .prepare_owned_volume_creation(&h.actor, &claim)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    let request = ResourceEffect {
        command: h.command,
        deployment_id: h.intent.id(),
        resource: fixtures::key("data"),
        expected_deployment_version: snapshot.version,
        expected_resource_version: snapshot.resources[&claim.resource].version,
        action: ResourceAction::Create,
        provenance: AttemptProvenance::new(
            &h.actor,
            DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
        )
        .unwrap(),
    };
    assert!(matches!(
        h.repository.claim_resource_effect(&h.actor, request).await,
        Err(DeploymentError::InvalidAction)
    ));
    assert_writer_denied(&h, &prepared).await;
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&manager, h.intent.id())
            .await
            .unwrap(),
        Some(prepared.preparation)
    );
    assert_eq!(
        before,
        claim_fixture::state(&pool).await,
        "closed Install cannot mutate preparation/claims/mapping or providers"
    );
}

async fn assert_writer_denied(
    h: &Harness,
    prepared: &recipe_application::ExecutionPreparationAdmission,
) {
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let allowed:bool=sqlx::query_scalar("SELECT recipe_execution_preparation_authorized(command) FROM recipe_deployment_commands command WHERE id=$1")
        .bind(h.command.id().as_uuid()).fetch_one(&mut *tx).await.unwrap();
    assert!(
        !allowed,
        "existing113 SQL closes the exact original Install independently of current lifecycle/source"
    );
    let children = prepared
        .preparation
        .instances()
        .iter()
        .map(|child| serde_json::to_vec(child).unwrap())
        .collect::<Vec<_>>();
    let error = sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
        .bind(h.intent.id().as_uuid())
        .bind(h.command.id().as_uuid())
        .bind(prepared.preparation.canonical_bytes().unwrap())
        .bind(children)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
}
