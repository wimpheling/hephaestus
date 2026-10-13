use identity_domain::RequestId;
use recipe_application::DeploymentError;
use sqlx::PgPool;

use crate::{fixture, support};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn fresh_target_authorization_precedes_source_facts_and_repeat_plan(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let planner = fixture::planner(&pool, 512).await;
    let request = fixture::request(&seeded, false);
    let mut actor = support::identity(seeded.maintainer);
    planner.plan(&actor, &request).await.unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seeded.consuming_project)
        .bind(actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    actor.request_id = RequestId::new();
    assert!(matches!(
        planner.plan(&actor, &request).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    let permissions: Vec<String> =
        sqlx::query_scalar("SELECT permission FROM authorization_audit_events WHERE request_id=$1")
            .bind(actor.request_id.as_uuid())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(permissions, ["can_manage"]);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn source_use_is_rechecked_without_historical_authority(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let planner = fixture::planner(&pool, 512).await;
    let request = fixture::request(&seeded, false);
    let actor = support::identity(seeded.maintainer);
    planner.plan(&actor, &request).await.unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(seeded.project)
        .bind(actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        planner.plan(&actor, &request).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
}
