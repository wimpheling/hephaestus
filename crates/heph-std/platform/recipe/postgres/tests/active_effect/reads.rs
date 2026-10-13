use recipe_application::{
    ActiveDeploymentEffectRepository, DeploymentError, DiagnosticCode, ResourceAction,
};
use sqlx::PgPool;

use crate::{
    claim_fixture, fixtures,
    harness::{Graph, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn original_and_new_request_load_exact_claim_without_rewriting_ambiguity_version(
    pool: PgPool,
) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    assert_eq!(
        claim_fixture::load(&h, &h.actor, h.command, &claim.resource)
            .await
            .unwrap(),
        Some(claim.clone())
    );
    h.repository
        .mark_resource_ambiguous(&h.actor, &claim, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .unwrap();
    let before = claim_fixture::state(&pool).await;
    let fresh = claim_fixture::fresh(&h.actor);
    let loaded = claim_fixture::load(&h, &fresh, h.command, &claim.resource)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded, claim);
    assert_ne!(loaded.provenance.request_id, fresh.request_id);
    let version:i64=sqlx::query_scalar("SELECT version FROM recipe_deployment_resources WHERE deployment_id=$1 AND resource_name=$2")
        .bind(h.intent.id().as_uuid()).bind(claim.resource.as_str()).fetch_one(&pool).await.unwrap();
    assert_eq!(u64::try_from(version).unwrap(), claim.resource_version + 1);
    assert_eq!(
        before,
        claim_fixture::state(&pool).await,
        "read creates no effect/progress/event/provider mutation"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn genuine_absence_is_none_without_new_claims_or_provider_rows(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    h.prepare().await;
    let before = claim_fixture::state(&pool).await;
    for name in ["data", "sqlite"] {
        assert!(
            claim_fixture::load(&h, &h.actor, h.command, &fixtures::key(name))
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(before, claim_fixture::state(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn absence_does_not_skip_complete_preparation_or_current_configuration(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    assert!(matches!(
        claim_fixture::load(&h, &h.actor, h.command, &fixtures::key("data")).await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    h.prepare().await;
    let before = claim_fixture::state(&pool).await;
    for configured in [
        h.configured(1024, "platform/preparation-v1"),
        h.configured(512, "changed/version"),
    ] {
        assert!(matches!(
            configured
                .load_active_effect(&h.actor, h.command, h.intent.id(), &fixtures::key("data"))
                .await,
            Err(DeploymentError::InputConflict)
        ));
    }
    assert_eq!(before, claim_fixture::state(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn reconciled_non_application_is_none_after_exact_worker_and_actor_transition(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    // This is a ledger-only fixture. A worker DTO is not physical creation proof.
    let worker = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
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
    recipe_postgres::PostgresEffectVerificationRecorder::new(worker.clone())
        .record(
            &claim,
            claim.provenance.attempt_id,
            recipe_postgres::VerifiedOutcome::NotApplied,
        )
        .await
        .unwrap();
    let request = claim_fixture::fresh(&h.actor);
    h.repository
        .record_reconciliation(
            &request,
            h.command,
            &claim,
            recipe_application::AttemptProvenance::new(
                &request,
                recipe_application::DeploymentAttemptId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
            )
            .unwrap(),
            recipe_application::ReconciledOutcome::NotApplied,
        )
        .await
        .unwrap_err();
    // Proof must name the actual reconciliation attempt, never borrowed completion.
    let attempt = recipe_application::DeploymentAttemptId::from_uuid(uuid::Uuid::new_v4()).unwrap();
    recipe_postgres::PostgresEffectVerificationRecorder::new(worker.clone())
        .record(
            &claim,
            attempt,
            recipe_postgres::VerifiedOutcome::NotApplied,
        )
        .await
        .unwrap();
    h.repository
        .record_reconciliation(
            &request,
            h.command,
            &claim,
            recipe_application::AttemptProvenance::new(&request, attempt).unwrap(),
            recipe_application::ReconciledOutcome::NotApplied,
        )
        .await
        .unwrap();
    let before = claim_fixture::state(&pool).await;
    assert!(
        claim_fixture::load(&h, &request, h.command, &claim.resource)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(before, claim_fixture::state(&pool).await);
    worker.close().await;
}
