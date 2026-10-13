use run_orchestrator::RunRepository;
use sqlx::PgPool;

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn actor_operation_and_raw_worker_receipt_forgery_are_denied(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let mut actor = h.producer.transaction().await;
    let error = h.write(&mut actor).await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"), "{error:?}");
    actor.rollback().await.unwrap();
    let mut worker = h.producer.activation.worker.begin().await.unwrap();
    let error = sqlx::query("INSERT INTO run_invocation_consumptions(run_id) VALUES($1)")
        .bind(h.start.run_id.as_uuid())
        .execute(&mut *worker)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"), "{error:?}");
    worker.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    let repo = h.repository();
    assert!(
        repo.get(h.start.run_id).await.is_err(),
        "115 tuple alone is not consumed history"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn protected_writer_transaction_rollback_leaves_start_unconsumed_and_original_run(
    pool: PgPool,
) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let mut worker = h.producer.activation.worker.begin().await.unwrap();
    h.write(&mut worker).await.unwrap();
    worker.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
    assert!(!h.consume().await.unwrap().created);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn deferred_worker_event_product_proof_is_required_for_commit(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let mut owner = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *owner)
        .await
        .unwrap();
    h.write(&mut owner).await.unwrap();
    // Privileged test fault injection removes only the newly returned product row.
    // No trigger/RLS/guard is disabled and the owning operation still used worker.
    sqlx::query("RESET ROLE")
        .execute(&mut *owner)
        .await
        .unwrap();
    let removed = sqlx::query("DELETE FROM product_event_outbox WHERE event_id=(SELECT event_id FROM run_invocation_consumptions WHERE run_id=$1)")
        .bind(h.start.run_id.as_uuid()).execute(&mut *owner).await.unwrap();
    assert_eq!(removed.rows_affected(), 1);
    let error = owner.commit().await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"), "{error:?}");
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn consumed_receipt_is_immutable_and_protected_catalog_is_not_actor_callable(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    h.consume().await.unwrap();
    let before = h.rows().await;
    let mut owner = pool.begin().await.unwrap();
    let error =
        sqlx::query("UPDATE run_invocation_consumptions SET consumed_at=now() WHERE run_id=$1")
            .bind(h.start.run_id.as_uuid())
            .execute(&mut *owner)
            .await
            .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
    owner.rollback().await.unwrap();
    let protected: bool = sqlx::query_scalar("SELECT c.relrowsecurity AND c.relforcerowsecurity AND NOT has_table_privilege('hephaestus_worker',c.oid,'INSERT') AND NOT has_table_privilege('hephaestus_app',c.oid,'INSERT') AND p.prosecdef AND p.proconfig @> ARRAY['search_path=pg_catalog, public, pg_temp'] AND pg_get_userbyid(p.proowner) NOT IN ('hephaestus_app','hephaestus_worker') AND NOT has_function_privilege('hephaestus_app',p.oid,'EXECUTE') FROM pg_class c JOIN pg_proc p ON p.oid='consume_qualified_invocation117(uuid,jsonb,bytea,text,text,text)'::regprocedure WHERE c.oid='run_invocation_consumptions'::regclass")
        .fetch_one(&pool).await.unwrap();
    assert!(
        protected,
        "fixed trusted owner/search path and forced RLS, no raw writer rights"
    );
    let mut actor = h.producer.transaction().await;
    let error = sqlx::query("SELECT inspect_qualified_invocation117($1)")
        .bind(h.start.run_id.as_uuid())
        .execute(&mut *actor)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"));
    actor.rollback().await.unwrap();
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn cancelled_start_is_not_an_empty_effect_or_new_consumption(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    sqlx::query("UPDATE runs SET cancel_requested_at=now() WHERE id=$1")
        .bind(h.start.run_id.as_uuid())
        .execute(&h.producer.activation.worker)
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(h.consume().await.is_err());
    assert_eq!(h.rows().await, before);
}

pub fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}
