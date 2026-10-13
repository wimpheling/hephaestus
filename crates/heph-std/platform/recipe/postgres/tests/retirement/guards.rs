use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use recipe_application::RetireExternalObservation;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{harness::Harness, helpers};

fn integrity(error: &sqlx::Error) {
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
}

async fn journal(
    tx: &mut Transaction<'_, Postgres>,
    h: &Harness,
    identity: &AuthenticatedIdentity,
    request: &RetireExternalObservation,
    forged_ready: bool,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v5(
        &request.provenance.attempt_id.as_uuid(),
        b"observation_retired",
    );
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(id.to_string())
        .execute(&mut **tx)
        .await?;
    let (event, cursor, aggregate): (Uuid, i64, i64) = sqlx::query_as(
        "SELECT event_id,cursor,aggregate_version FROM append_application_event(
        $1,'project',$2,'project',$2,'project.changed','updated',NULL,$3,NULL)",
    )
    .bind(id)
    .bind(h.intent.project_id().as_uuid())
    .bind(h.fixture.organization)
    .fetch_one(&mut **tx)
    .await?;
    let mut after: Value = sqlx::query_scalar(
        "SELECT recipe_effect_resource_state(resource) FROM recipe_deployment_resources resource
        WHERE deployment_id=$1 AND resource_name=$2",
    )
    .bind(request.target.deployment_id.as_uuid())
    .bind(request.target.resource.as_str())
    .fetch_one(&mut **tx)
    .await?;
    after["active_attempt"] = Value::Null;
    after["version"] = serde_json::json!(after["version"].as_i64().expect("version") + 1);
    if forged_ready {
        after["install"] = serde_json::json!("ready");
    }
    sqlx::query("INSERT INTO recipe_execution_transitions (id,command_id,attempt_id,operation_id,deployment_id,project_id,
        resource_name,kind,fingerprint,actor_id,request_id,before_state,after_state,before_deployment_version,
        after_deployment_version,before_lifecycle,after_lifecycle,event_id,event_cursor,event_aggregate_version)
        SELECT $1,$2,$3,$4,deployment.id,deployment.project_id,resource.resource_name,'observation_retired',$5,$6,$7,
            recipe_effect_resource_state(resource),$8,deployment.version,deployment.version+1,deployment.lifecycle,'removing',$9,$10,$11
        FROM recipe_deployments deployment JOIN recipe_deployment_resources resource ON resource.deployment_id=deployment.id
        WHERE deployment.id=$12 AND resource.resource_name=$13")
        .bind(id).bind(request.command.id().as_uuid()).bind(request.target.provenance.attempt_id.as_uuid())
        .bind(request.provenance.attempt_id.as_uuid()).bind(vec![0_u8;32]).bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid()).bind(sqlx::types::Json(after)).bind(event).bind(cursor).bind(aggregate)
        .bind(request.target.deployment_id.as_uuid()).bind(request.target.resource.as_str()).execute(&mut **tx).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn sql_retirement_cannot_assert_ready_or_commit_an_unapplied_journal(pool: PgPool) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, true).await;
    let target = helpers::claim(&h, true).await;
    let cleanup = crate::support::identity(h.fixture.maintainer);
    let retirement = helpers::admit_cleanup(&h, &cleanup, &target).await;
    let before = helpers::state(&pool, &target).await;
    let mut tx = begin_actor_transaction(&h.app, &cleanup)
        .await
        .expect("actor transaction");
    integrity(
        &journal(&mut tx, &h, &cleanup, &retirement, true)
            .await
            .expect_err("no verification-success assertion"),
    );
    tx.rollback().await.expect("rollback forgery");
    let mut tx = begin_actor_transaction(&h.app, &cleanup)
        .await
        .expect("actor transaction");
    journal(&mut tx, &h, &cleanup, &retirement, false)
        .await
        .expect("provisional exact retirement");
    integrity(
        &tx.commit()
            .await
            .expect_err("journal must atomically update actual state"),
    );
    assert_eq!(before, helpers::state(&pool, &target).await);
    let mut tx = begin_actor_transaction(&h.app, &cleanup)
        .await
        .expect("actor transaction");
    integrity(&sqlx::query("UPDATE recipe_deployment_resources SET active_attempt_id=NULL,version=version+1 WHERE deployment_id=$1")
        .bind(h.intent.id().as_uuid()).execute(&mut *tx).await.expect_err("no bypass without exact journal"));
    tx.rollback().await.expect("rollback bare retirement");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn sql_retirement_never_releases_an_owned_creation_fence(pool: PgPool) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, false).await;
    let target = helpers::claim(&h, false).await;
    let cleanup = crate::support::identity(h.fixture.maintainer);
    let retirement = helpers::admit_cleanup(&h, &cleanup, &target).await;
    let mut tx = begin_actor_transaction(&h.app, &cleanup)
        .await
        .expect("actor transaction");
    integrity(
        &journal(&mut tx, &h, &cleanup, &retirement, false)
            .await
            .expect_err("owned mutation cannot be retired"),
    );
    tx.rollback().await.expect("rollback owned fence forgery");
    assert_eq!(
        helpers::state(&pool, &target).await["active_attempt"],
        serde_json::json!(target.provenance.attempt_id.as_uuid())
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn sql_retirement_requires_an_admitted_removal_and_cannot_cancel_installation_work(
    pool: PgPool,
) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, true).await;
    let target = helpers::claim(&h, true).await;
    let request = helpers::retirement(&h, &h.identity, h.command, &target).await;
    let mut tx = begin_actor_transaction(&h.app, &h.identity)
        .await
        .expect("original actor transaction");
    integrity(
        &journal(&mut tx, &h, &h.identity, &request, false)
            .await
            .expect_err("installation has no retirement authority"),
    );
    tx.rollback()
        .await
        .expect("rollback pre-removal retirement");
    assert_eq!(
        helpers::state(&pool, &target).await["active_attempt"],
        serde_json::json!(target.provenance.attempt_id.as_uuid())
    );
}
