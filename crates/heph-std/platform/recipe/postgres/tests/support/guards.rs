use authz_postgres::begin_actor_transaction;
use recipe_postgres::PostgresDeploymentRepository;
use sqlx::PgPool;

use crate::{fixtures, seed, support};

fn integrity(error: &sqlx::Error) {
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn forced_rls_immutable_intent_and_audited_commit_guards_hold(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let app = fixtures::app_pool(&pool).await;
    let repository = PostgresDeploymentRepository::new(app.clone());
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, false);
    let admission = repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("admission");
    let rls: (i64, i64) = sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE relrowsecurity AND relforcerowsecurity)
        FROM pg_class WHERE relname IN ('recipe_definitions', 'recipe_deployments', 'recipe_deployment_resources',
            'recipe_deployment_commands', 'recipe_deployment_admission_attempts')")
        .fetch_one(&pool).await.expect("forced RLS catalog");
    assert_eq!(rls, (5, 5));
    let outsider = support::identity(fixture.outsider);
    let mut tx = begin_actor_transaction(&app, &outsider)
        .await
        .expect("outsider transaction");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_deployments")
        .fetch_one(&mut *tx)
        .await
        .expect("RLS read");
    assert_eq!(count, 0);
    tx.rollback().await.expect("rollback read");
    integrity(
        &sqlx::query("UPDATE recipe_deployments SET deployment_key = 'reused' WHERE id = $1")
            .bind(intent.id().as_uuid())
            .execute(&pool)
            .await
            .expect_err("immutable key"),
    );
    integrity(
        &sqlx::query("DELETE FROM recipe_deployments WHERE id = $1")
            .bind(intent.id().as_uuid())
            .execute(&pool)
            .await
            .expect_err("no tombstone erasure"),
    );
    integrity(
        &sqlx::query(
            "UPDATE recipe_deployment_commands SET event_cursor = event_cursor + 1 WHERE id = $1",
        )
        .bind(admission.receipt.command.id().as_uuid())
        .execute(&pool)
        .await
        .expect_err("receipt write once"),
    );
    integrity(
        &sqlx::query("DELETE FROM recipe_deployment_admission_attempts WHERE deployment_id = $1")
            .bind(intent.id().as_uuid())
            .execute(&pool)
            .await
            .expect_err("append-only provenance"),
    );
    integrity(&sqlx::query("UPDATE recipe_deployment_resources SET install_progress = 'ready', version = version + 1 WHERE deployment_id = $1")
        .bind(intent.id().as_uuid()).execute(&pool).await.expect_err("effects unavailable until claim guards exist"));
    let mut tx = begin_actor_transaction(&app, &identity)
        .await
        .expect("actor transaction");
    sqlx::query(
        "UPDATE recipe_deployments SET lifecycle = 'removing', version = version + 1 WHERE id = $1",
    )
    .bind(intent.id().as_uuid())
    .execute(&mut *tx)
    .await
    .expect("provisional transition");
    integrity(
        &tx.commit()
            .await
            .expect_err("commit requires matching audited receipt/outbox"),
    );
    assert_eq!(
        repository
            .inspect(&identity, intent.id())
            .await
            .expect("rollback state")
            .version,
        0
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn cleanup_rejects_existing_predicted_identity_owned_by_another_project(pool: PgPool) {
    let fixture = seed::seed(&pool).await;
    let repository = PostgresDeploymentRepository::new(fixtures::app_pool(&pool).await);
    let identity = support::identity(fixture.maintainer);
    let intent = fixtures::intent(&fixture, false);
    repository
        .admit_install(&identity, fixtures::install(&identity, intent.clone()))
        .await
        .expect("intent admission");
    let recipe_application::PlannedResourceIdentity::Instance { id, .. } =
        intent.resources()[&fixtures::key("sqlite")].identity()
    else {
        panic!("instance plan")
    };
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
        SELECT $1, $2, family_id, 'different-owner', 'active' FROM agent_instances WHERE id = $3",
    )
    .bind(id.as_uuid())
    .bind(fixture.project)
    .bind(fixture.instance)
    .execute(&pool)
    .await
    .expect("actual conflicting identity");
    let cleanup = support::identity(fixture.maintainer);
    assert!(matches!(
        repository
            .admit_remove(&cleanup, fixtures::remove(&cleanup, intent.id(), 0))
            .await,
        Err(recipe_application::DeploymentError::IntentMismatch)
    ));
    assert_eq!(
        repository
            .inspect(&identity, intent.id())
            .await
            .expect("unchanged ledger")
            .version,
        0
    );
}
