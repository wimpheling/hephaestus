use authz_postgres::begin_actor_transaction;
use recipe_application::ResourceAction;
use sqlx::PgPool;

use crate::{
    claim_fixture,
    harness::{Graph, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn raw_stale_generation_foreign_identity_and_orphan_attempt_cannot_change_loaded_claim(
    pool: PgPool,
) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    let before = claim_fixture::state(&pool).await;
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error=sqlx::query("UPDATE recipe_deployment_resources SET execution_generation=execution_generation+1 WHERE deployment_id=$1 AND resource_name=$2")
        .bind(h.intent.id().as_uuid()).bind(claim.resource.as_str()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(code(&error), Some("23000".to_owned()), "{error:?}");
    tx.rollback().await.unwrap();
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error=sqlx::query("UPDATE recipe_effect_attempts SET claim_json=jsonb_set(claim_json,'{identity,id}',to_jsonb($2::text)) WHERE id=$1")
        .bind(claim.provenance.attempt_id.as_uuid()).bind(uuid::Uuid::new_v4().to_string()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(code(&error), Some("42501".to_owned()));
    tx.rollback().await.unwrap();
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error=sqlx::query("UPDATE recipe_deployment_resources SET active_attempt_id=$3 WHERE deployment_id=$1 AND resource_name=$2")
        .bind(h.intent.id().as_uuid()).bind(claim.resource.as_str()).bind(uuid::Uuid::new_v4()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(code(&error), Some("23000".to_owned()));
    tx.rollback().await.unwrap();
    assert_eq!(
        claim_fixture::load(&h, &h.actor, h.command, &claim.resource)
            .await
            .unwrap(),
        Some(claim)
    );
    assert_eq!(before, claim_fixture::state(&pool).await);
}

fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}
