use std::time::Duration;

use hephaestus_app::recipe_instance_import::{InstanceImportError, InstanceImportFailure};
use identity_domain::RequestId;
use recipe_application::{DeploymentError, InstallProgress};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{failures, fixtures, instance_fixture::Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn cancellation_during_actual_import_keeps_durable_claim_for_fresh_idempotent_retry(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prepared().await;
    let claim = h.claim().await;
    let (blocker, pid) = block_insertion(&pool).await;
    let pending = spawn_import(&h);
    wait_for_insertion(&pool, pid).await;
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    blocker.commit().await.unwrap();
    assert_eq!(h.active().await, Some(claim.clone()));
    let facts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_instances WHERE id=$1),
      (SELECT count(*) FROM release_command_inbox WHERE command_key=$2)",
    )
    .bind(prepared.instance_id().as_uuid())
    .bind(prepared.command_key().as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        facts,
        (0, 0),
        "this actual cancelled transaction committed no import; no absence receipt is inferred"
    );
    let mut fresh = h.actor.clone();
    fresh.request_id = RequestId::new();
    let result = h
        .importer()
        .import_prepared_instance(&fresh, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_effect_attempts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "retry retains the original attempt and generation"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn primary_actual_service_failure_survives_source_loss_and_failed_ambiguity_recording(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prepared().await;
    let claim = h.claim().await;
    failures::fail_revisions(&pool).await;
    let (blocker, pid) = block_insertion(&pool).await;
    let pending = spawn_import(&h);
    wait_for_insertion(&pool, pid).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    blocker.commit().await.unwrap();
    assert!(
        matches!(
            pending.await.unwrap(),
            Err(InstanceImportError::Held {
                failure: InstanceImportFailure::Release(_),
                ambiguity_error: Some(DeploymentError::AuthorizationDenied)
            })
        ),
        "the real service failure remains primary when holding loses current source permission"
    );
    let facts: (uuid::Uuid, i64, i64) = sqlx::query_as(
        "SELECT resource.active_attempt_id,
      (SELECT count(*) FROM agent_instances WHERE id=$2),
      (SELECT count(*) FROM recipe_effect_verifications WHERE outcome='not_applied')
      FROM recipe_deployment_resources resource WHERE deployment_id=$1 AND resource_name='sqlite'",
    )
    .bind(h.intent.id().as_uuid())
    .bind(prepared.instance_id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(facts, (claim.provenance.attempt_id.as_uuid(), 0, 0));
}

pub fn spawn_import(
    h: &Harness,
) -> tokio::task::JoinHandle<Result<recipe_application::ResourceProgress, InstanceImportError>> {
    let importer = h.importer();
    let actor = h.actor.clone();
    let command = h.command;
    let deployment = h.intent.id();
    tokio::spawn(async move {
        importer
            .import_prepared_instance(&actor, command, deployment, &fixtures::key("sqlite"))
            .await
    })
}

pub async fn block_insertion(pool: &PgPool) -> (Transaction<'_, Postgres>, i32) {
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    // Reads and metadata admission stay available; the actual service's first
    // INSERT is the deterministic pause, before any instance row can exist.
    sqlx::query("LOCK TABLE agent_instances IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .unwrap();
    (tx, pid)
}

pub async fn wait_for_insertion(pool: &PgPool, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity
              WHERE activity.datname=current_database() AND activity.pid<>pg_backend_pid()
                AND pg_blocking_pids(activity.pid) @> ARRAY[$1::integer]
                AND activity.query LIKE 'INSERT INTO agent_instances(id,project_id,%')",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actual import must reach the exact table lock barrier");
}
