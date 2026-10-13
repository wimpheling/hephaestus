use super::fixture::Closure;
use authz_postgres::PostgresMelangeAuthorizer;
use recipe_application::{
    DeploymentError, PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
};
use release_postgres::{ReleaseService, ReleaseServiceError};
use release_service::RequestInstanceRemoval;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};

#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn owning_closure_waits_on_original_reservation_before_any_parent_lock(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let request = f.request().await;
    let app = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let actor_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&app)
        .await
        .unwrap();
    let service = f
        .original
        .configured(app.clone(), f.original.base.worker.clone());
    let key = i64::from_be_bytes(
        *f.original
            .base
            .input
            .command_key()
            .as_bytes()
            .first_chunk::<8>()
            .unwrap(),
    );
    let mut blocker = pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let manager = f.manager.clone();
    let owned_request = request.clone();
    let pending =
        tokio::spawn(async move { service.close_for_remove(&manager, &owned_request).await });
    wait_on(&pool, actor_pid, blocker_pid).await;
    // Correlated pg_blocking_pids + advisory wait, not elapsed scheduler timing.
    assert_eq!(f.snapshot().await, before);
    let mut probe = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(f.original.base.intent.project_id().as_uuid())
        .execute(&mut *probe)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(f.original.base.input.instance_id().as_uuid())
        .execute(&mut *probe)
        .await
        .unwrap();
    probe.rollback().await.unwrap();
    blocker.commit().await.unwrap();
    assert!(!pending.await.unwrap().unwrap().replayed);
    app.close().await;
}
async fn wait_on(pool: &PgPool, waiting: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let observed:bool=sqlx::query_scalar("SELECT $2=ANY(pg_blocking_pids($1)) AND EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND wait_event='advisory')")
                .bind(waiting).bind(blocker).fetch_one(pool).await.unwrap();
            if observed {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("exact actor must wait on held original birth mutex");
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn late_instance_contention_fails_nowait_without_partial_closure_or_cancel(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let request = f.request().await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(f.original.base.input.instance_id().as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let error = raw_qualify(&f, &request).await.unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03")
    );
    assert!(matches!(
        f.original
            .service()
            .close_for_remove(&f.manager, &request)
            .await,
        Err(DeploymentError::Unavailable)
    ));
    assert_eq!(f.snapshot().await, before);
    blocker.rollback().await.unwrap();
    f.original
        .service()
        .close_for_remove(&f.manager, &request)
        .await
        .unwrap();
}
async fn raw_qualify(
    f: &Closure,
    request: &PreparedInstanceClosureRequest,
) -> Result<bool, sqlx::Error> {
    // Admin reads are fixture diagnostics of genuine existing rows, not proofs
    // created by the fixture. The facade runs under the actual current actor.
    let (whole,original,drain):(Vec<u8>,serde_json::Value,serde_json::Value)=sqlx::query_as("SELECT preparation.canonical_input,original.claim_json,drain.claim_json FROM recipe_deployment_execution_preparations preparation JOIN recipe_effect_attempts original ON original.id=$1 JOIN recipe_effect_attempts drain ON drain.id=$2 WHERE preparation.deployment_id=$3")
        .bind(request.target().original().provenance.attempt_id.as_uuid()).bind(request.drain().provenance.attempt_id.as_uuid()).bind(request.intent().id().as_uuid()).fetch_one(&f.original.pool).await?;
    let hash:[u8;32]=Sha256::digest(serde_json::to_vec(&json!({"version":1,"operation":"request_instance_removal","actor":f.manager.user_id,"instance":request.target().prepared().instance_id(),"removal":request.removal_id(),"expected_version":request.expected_instance_version()})).unwrap()).into();
    let mut tx = authz_postgres::begin_actor_transaction(&f.original.base.app, &f.manager).await?;
    let result = sqlx::query_scalar(
        "SELECT prepare_prepared_instance_closure118($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(request.target().original().provenance.attempt_id.as_uuid())
    .bind(original)
    .bind(serde_json::to_value(request.target().prepared()).unwrap())
    .bind(whole)
    .bind(drain)
    .bind(request.command_key().as_bytes().as_slice())
    .bind(request.removal_id().as_uuid())
    .bind(i64::try_from(request.expected_instance_version()).unwrap())
    .bind(hash.as_slice())
    .fetch_one(&mut *tx)
    .await;
    tx.rollback().await?;
    result
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn raw_generic_closure_cannot_wait_on_prepared_parent_after_child_lock(pool: PgPool) {
    let f = Closure::new(&pool, true).await;
    let request = f.request().await;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE")
        .bind(f.original.base.intent.project_id().as_uuid())
        .execute(&mut *blocker)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let service = ReleaseService::new(
        f.original.base.app.clone(),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let result = service
        .request_instance_removal(
            &f.manager,
            RequestInstanceRemoval {
                command_key: request.command_key(),
                removal_id: request.removal_id(),
                instance_id: f.original.base.input.instance_id(),
                expected_version: request.expected_instance_version(),
            },
        )
        .await;
    let Err(ReleaseServiceError::Database(error)) = result else {
        panic!("raw prepared closure must fail with bounded row-lock contention");
    };
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03")
    );
    assert_eq!(
        f.snapshot().await,
        before,
        "no removal/inbox/outbox/event/audit partial writes"
    );
    blocker.rollback().await.unwrap();
    f.original.close(&f.manager).await;
}
