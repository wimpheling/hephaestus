use authz_postgres::begin_actor_transaction;
use recipe_application::ResourceAction;
use sqlx::PgPool;
use uuid::Uuid;

use crate::harness::Harness;

fn code(error: &sqlx::Error) -> Option<std::borrow::Cow<'_, str>> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn actor_cannot_insert_worker_proof_or_change_progress_without_exact_journal(pool: PgPool) {
    let h = Harness::new(&pool, false, false).await;
    let mut tx = begin_actor_transaction(&h.app, &h.identity)
        .await
        .expect("actor transaction");
    let error = sqlx::query("INSERT INTO recipe_effect_verifications DEFAULT VALUES")
        .execute(&mut *tx)
        .await
        .expect_err("actor has no worker proof privilege");
    assert_eq!(code(&error).as_deref(), Some("42501"));
    tx.rollback().await.expect("rollback proof forgery");
    let mut tx = begin_actor_transaction(&h.app, &h.identity)
        .await
        .expect("actor transaction");
    let error = sqlx::query("UPDATE recipe_deployment_resources SET install_progress='ready', version=version+1 WHERE deployment_id=$1")
        .bind(h.intent.id().as_uuid()).execute(&mut *tx).await.expect_err("journal required");
    assert_eq!(code(&error).as_deref(), Some("23000"));
    tx.rollback().await.expect("rollback progress forgery");
    let rls: (i64, i64) = sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE relrowsecurity AND relforcerowsecurity)
        FROM pg_class WHERE relname IN ('recipe_effect_attempts','recipe_effect_verifications','recipe_execution_transitions','recipe_command_results')")
        .fetch_one(&pool).await.expect("execution forced RLS");
    assert_eq!(rls, (4, 4));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn bare_valid_attempt_cannot_commit_without_audited_event_and_applied_claim_transition(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false, false).await;
    let mut tx = begin_actor_transaction(&h.app, &h.identity)
        .await
        .expect("actor transaction");
    let attempt = Uuid::new_v4();
    sqlx::query("INSERT INTO recipe_effect_attempts (id,command_id,deployment_id,project_id,resource_name,action,generation,input_hash,
        claim_json,before_state,request_fingerprint,expected_deployment_version,actor_id,request_id)
        SELECT $1,$2,resource.deployment_id,resource.project_id,resource.resource_name,'create',1,resource.input_hash,
            jsonb_build_object('command_id',$2::uuid,'deployment_id',resource.deployment_id,'resource',resource.resource_name,
                'identity',resource.plan_json->'identity','input_hash',recipe_effect_hash_json(resource.input_hash),
                'resource_version',1,'generation',1,'action','create','attempt_id',$1::uuid,'actor_id',$3::uuid,'request_id',$4::uuid),
            recipe_effect_resource_state(resource),resource.input_hash,0,$3,$4
        FROM recipe_deployment_resources resource WHERE resource.deployment_id=$5 AND resource.resource_name='data'")
        .bind(attempt).bind(h.command.id().as_uuid()).bind(h.identity.user_id.as_uuid())
        .bind(h.identity.request_id.as_uuid()).bind(h.intent.id().as_uuid()).execute(&mut *tx).await.expect("valid provisional header");
    let error = tx
        .commit()
        .await
        .expect_err("unapplied claim cannot commit");
    assert_eq!(code(&error).as_deref(), Some("23000"));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_effect_attempts WHERE id=$1")
        .bind(attempt)
        .fetch_one(&pool)
        .await
        .expect("atomic rollback");
    assert_eq!(count, 0);
    assert_eq!(
        h.repository
            .inspect(&h.identity, h.intent.id())
            .await
            .expect("unchanged deployment")
            .version,
        0
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn attempts_verifications_transitions_and_terminal_results_are_append_only(pool: PgPool) {
    let h = Harness::new(&pool, false, false).await;
    h.apply(&h.request(), h.command, "data", ResourceAction::Create)
        .await;
    let snapshot = h
        .repository
        .inspect(&h.identity, h.intent.id())
        .await
        .expect("ready");
    h.repository
        .finish_command(&h.identity, h.command, h.intent.id(), snapshot.version)
        .await
        .expect("terminal");
    for query in [
        sqlx::query("DELETE FROM recipe_effect_attempts"),
        sqlx::query("DELETE FROM recipe_effect_verifications"),
        sqlx::query("DELETE FROM recipe_execution_transitions"),
        sqlx::query("DELETE FROM recipe_command_results"),
    ] {
        let error = query.execute(&pool).await.expect_err("append-only ledger");
        assert_eq!(code(&error).as_deref(), Some("23000"));
    }
    let error = sqlx::query("UPDATE recipe_command_results SET event_cursor=event_cursor+1")
        .execute(&pool)
        .await
        .expect_err("terminal receipt immutable");
    assert_eq!(code(&error).as_deref(), Some("23000"));
}
