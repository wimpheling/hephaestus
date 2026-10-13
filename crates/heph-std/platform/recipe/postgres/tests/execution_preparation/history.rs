use authz_postgres::begin_actor_transaction;
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError,
    DeploymentExecutionPreparationRepository, DeploymentOperation, ResourceAction, ResourceEffect,
};
use sqlx::PgPool;

use crate::{
    fixtures,
    harness::{self, Graph, Harness},
    schema, support,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn malformed_extra_install_is_held_before_first_preparation(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    extra_install(&h).await;
    let before = harness::evidence(&pool).await;
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::IntentMismatch)
    ));
    assert_eq!(before, harness::evidence(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn extra_install_after_commit_cannot_replay_or_claim_with_old_header(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let first = h.prepare().await;
    extra_install(&h).await;
    let before = harness::evidence(&pool).await;
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::IntentMismatch)
    ));
    let effect = creation(&h);
    assert!(
        h.repository
            .claim_resource_effect(&h.actor, effect)
            .await
            .is_err()
    );
    assert_eq!(before, harness::evidence(&pool).await);
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&h.actor, h.intent.id())
            .await
            .unwrap(),
        Some(first.preparation),
        "immutable history remains data even when new execution is held"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn genuine_112_to_113_preserves_history_checksums_and_holds_preexisting_claim(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 112).await;
    let claim = h
        .repository
        .claim_resource_effect(&h.actor, creation(&h))
        .await
        .unwrap();
    assert_eq!(claim.generation, 1);
    let before = historical(&pool).await;
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=112 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    schema::checkpoint(113).run(&pool).await.unwrap();
    assert_eq!(before, historical(&pool).await);
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=112 ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(checksums, after);
    let marker: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(marker, 113);
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    assert!(
        h.preparation
            .inspect_execution_preparation(&h.actor, h.intent.id())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(before, historical(&pool).await);
    eprintln!(
        "REAL_EXECUTION_PREPARATION_UPGRADE=1 historical=112 target=113 checksums_preserved=1 no_backfill=1 database_only=1"
    );
}

pub fn creation(h: &Harness) -> ResourceEffect {
    ResourceEffect {
        command: h.command,
        deployment_id: h.intent.id(),
        resource: fixtures::key("data"),
        expected_deployment_version: 0,
        expected_resource_version: 0,
        action: ResourceAction::Create,
        provenance: AttemptProvenance::new(
            &h.actor,
            DeploymentAttemptId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
        )
        .unwrap(),
    }
}

async fn extra_install(h: &Harness) {
    let actor = support::identity(h.actor.user_id);
    let command = CommandIdentity::from_identity(&actor, DeploymentOperation::Install).unwrap();
    let mut tx = begin_actor_transaction(&h.app, &actor).await.unwrap();
    let (event, cursor, version): (uuid::Uuid, i64, i64) = sqlx::query_as(
        "SELECT event_id,cursor,aggregate_version FROM append_application_event($1,'project',$2,
          'project',$2,'project.changed','updated',NULL,$3,NULL)",
    )
    .bind(command.idempotency_id().as_uuid())
    .bind(h.fixture.consuming_project)
    .bind(h.fixture.organization)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO recipe_deployment_commands(id,deployment_id,project_id,actor_id,operation,
      idempotency_id,input_hash,first_request_id,receipt_lifecycle,receipt_version,event_id,event_cursor,event_aggregate_version)
      VALUES($1,$2,$3,$4,'install',$5,$6,$7,'installing',0,$8,$9,$10)")
      .bind(command.id().as_uuid()).bind(h.intent.id().as_uuid()).bind(h.fixture.consuming_project)
      .bind(actor.user_id.as_uuid()).bind(command.idempotency_id().as_uuid())
      .bind(h.intent.input_hash().as_bytes().as_slice()).bind(actor.request_id.as_uuid())
      .bind(event).bind(cursor).bind(version).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

async fn historical(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'definitions',(SELECT jsonb_agg(to_jsonb(row) ORDER BY project_id,recipe_id,recipe_version) FROM recipe_definitions row),
      'deployments',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployments row),
      'resources',(SELECT jsonb_agg(to_jsonb(row) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources row),
      'commands',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_deployment_commands row),
      'claims',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM recipe_effect_attempts row),
      'volumes',(SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM agent_instance_state_volumes row),
      'purposes',(SELECT jsonb_agg(to_jsonb(row) ORDER BY volume_id) FROM volume_owned_backing_purposes row))")
      .fetch_one(pool).await.unwrap()
}
