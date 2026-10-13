use identity_domain::RequestId;
use recipe_application::{DeploymentError, DeploymentExecutionPreparationRepository};
use sqlx::PgPool;

use crate::harness::{self, Graph, Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn complete_two_imports_commit_exact_original_fields_and_fresh_request(pool: PgPool) {
    let h = Harness::new(&pool, Graph::TwoInstances, 113).await;
    let mut actor = h.actor.clone();
    actor.request_id = RequestId::new();
    let result = h
        .preparation
        .prepare_execution(&actor, h.command, h.intent.id())
        .await
        .unwrap();
    assert!(!result.replayed);
    assert_eq!(result.preparation, h.input());
    assert_eq!(result.preparation.instances().len(), 2);
    assert_eq!(result.receipt.preparation_request_id, actor.request_id);
    let provenance: (uuid::Uuid,uuid::Uuid,uuid::Uuid) = sqlx::query_as(
        "SELECT deployment.first_request_id,header.request_id,event.request_id
         FROM recipe_deployment_execution_preparations header JOIN recipe_deployments deployment
           ON deployment.id=header.deployment_id JOIN application_events event ON event.id=header.event_id
         WHERE header.deployment_id=$1",
    ).bind(h.intent.id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        provenance,
        (
            h.actor.request_id.as_uuid(),
            actor.request_id.as_uuid(),
            actor.request_id.as_uuid()
        )
    );
    let persisted: Vec<u8> = sqlx::query_scalar("SELECT canonical_input FROM recipe_deployment_execution_preparations WHERE deployment_id=$1")
        .bind(h.intent.id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(persisted, result.preparation.canonical_bytes().unwrap());
    let empty: (i64,i64,i64,i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recipe_effect_attempts),(SELECT count(*) FROM recipe_owned_volume_preparations),
          (SELECT count(*) FROM agent_instance_volume_mount_grants),
          (SELECT count(*) FROM agent_instances WHERE id=ANY($1))",
    ).bind(result.preparation.instances().iter().map(|i|i.instance_id().as_uuid()).collect::<Vec<_>>())
      .fetch_one(&pool).await.unwrap();
    assert_eq!(
        empty,
        (0, 0, 0, 0),
        "preparation has no side effects or grants"
    );
    let outbox: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM product_event_outbox WHERE event_id=$1)")
            .bind(result.receipt.event.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(outbox);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn concurrent_retries_have_one_graph_original_receipt_and_fresh_audits(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let mut left = h.actor.clone();
    left.request_id = RequestId::new();
    let mut right = h.actor.clone();
    right.request_id = RequestId::new();
    let (left_result, right_result) = tokio::join!(
        h.preparation
            .prepare_execution(&left, h.command, h.intent.id()),
        h.preparation
            .prepare_execution(&right, h.command, h.intent.id()),
    );
    let left_result = left_result.unwrap();
    let right_result = right_result.unwrap();
    assert_ne!(left_result.replayed, right_result.replayed);
    assert_eq!(left_result.preparation, right_result.preparation);
    assert_eq!(left_result.receipt, right_result.receipt);
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recipe_deployment_execution_preparations),
          (SELECT count(*) FROM recipe_prepared_instance_imports),
          (SELECT count(DISTINCT request_id) FROM authorization_audit_events
             WHERE request_id=ANY($1) AND decision='allow' AND permission='can_use')",
    )
    .bind(vec![left.request_id.as_uuid(), right.request_id.as_uuid()])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (1, 1, 2));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn policy_and_version_drift_hold_without_rewriting_frozen_input(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    h.prepare().await;
    let before = harness::evidence(&pool).await;
    for adapter in [
        h.configured(1024, "platform/preparation-v1"),
        h.configured(512, "platform/preparation-v2"),
    ] {
        assert!(matches!(
            adapter
                .prepare_execution(&h.actor, h.command, h.intent.id())
                .await,
            Err(DeploymentError::InputConflict)
        ));
    }
    assert_eq!(before, harness::evidence(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn zero_imports_still_freeze_global_platform_and_reject_drift(pool: PgPool) {
    let h = Harness::new(&pool, Graph::ZeroInstances, 113).await;
    let first = h.prepare().await;
    assert!(first.preparation.instances().is_empty());
    let persisted: (i32,serde_json::Value,i64) = sqlx::query_as(
        "SELECT instance_count,platform_json,(SELECT count(*) FROM recipe_prepared_instance_imports)
         FROM recipe_deployment_execution_preparations WHERE deployment_id=$1",
    ).bind(h.intent.id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(persisted.0, 0);
    assert_eq!(persisted.2, 0);
    assert_eq!(
        persisted.1,
        serde_json::to_value(first.preparation.platform()).unwrap()
    );
    assert!(matches!(
        h.configured(512, "changed-global-version")
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::InputConflict)
    ));
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&h.actor, h.intent.id())
            .await
            .unwrap(),
        Some(first.preparation)
    );
}
