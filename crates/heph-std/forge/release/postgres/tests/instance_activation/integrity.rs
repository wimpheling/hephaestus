use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use release_service::{
    InstanceExecutionError, InstanceExecutionService, InstanceInvocationId, InvokeInstance,
};
use runtime_types::{CommandId, RunId};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use super::{behavior::code, fixture::Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn raw_gate_worker_forgery_historical_birth_and_invocation_remain_denied(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let before = h.rows().await;
    let mut actor = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error = sqlx::query("UPDATE agent_instances SET run_gate_open=true,version=2 WHERE id=$1")
        .bind(h.command.instance_id().as_uuid())
        .execute(&mut *actor)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
    actor.rollback().await.unwrap();
    let worker = PgPoolOptions::new()
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let error =
        sqlx::query("SELECT * FROM write_qualified_instance_activation($1,$2,$3,$4,$5,$6,$7)")
            .bind(h.command.activation_id().as_uuid())
            .bind(h.command.command_key().as_bytes().as_slice())
            .bind(h.command.instance_id().as_uuid())
            .bind(h.command.expected_revision_id().as_uuid())
            .bind(1_i64)
            .bind(&h.config_wire)
            .bind(AUTHORIZATION_MODEL_VERSION)
            .execute(&worker)
            .await
            .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"));
    let mut actor = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error = sqlx::query("INSERT INTO agent_instance_execution_activations(id) VALUES($1)")
        .bind(Uuid::new_v4())
        .execute(&mut *actor)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"));
    actor.rollback().await.unwrap();
    let mut actor = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    let error = sqlx::query("SELECT * FROM qualified_instance_activation_context($1)")
        .bind(historical_instance(&pool, &h).await)
        .execute(&mut *actor)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("P1140"));
    actor.rollback().await.unwrap();
    let invocation = InvokeInstance::new(
        h.command.command_key(),
        InstanceInvocationId::from_uuid(Uuid::new_v4()).unwrap(),
        RunId::from_uuid(Uuid::new_v4()),
        CommandId::from_uuid(Uuid::new_v4()),
        h.command.instance_id(),
        h.command.expected_revision_id(),
    )
    .unwrap();
    assert!(matches!(
        h.service().invoke_instance(&h.actor, invocation).await,
        Err(InstanceExecutionError::Unsupported)
    ));
    assert_eq!(h.rows().await, before);
    let protected:bool=sqlx::query_scalar("SELECT (SELECT bool_and(relrowsecurity AND relforcerowsecurity) FROM pg_class WHERE oid IN ('agent_instance_execution_activations'::regclass,'agent_instance_execution_activation_audits'::regclass)) AND NOT has_table_privilege('hephaestus_app','agent_instance_execution_activations','INSERT') AND NOT has_function_privilege('hephaestus_worker','write_qualified_instance_activation(uuid,bytea,uuid,uuid,bigint,bytea,text)','EXECUTE') AND EXISTS(SELECT 1 FROM pg_proc WHERE oid='write_qualified_instance_activation(uuid,bytea,uuid,uuid,bigint,bytea,text)'::regprocedure AND prosecdef AND proconfig @> ARRAY['search_path=pg_catalog, public, pg_temp'])").fetch_one(&pool).await.unwrap();
    assert!(protected, "protected writer, exact ACL and FORCE RLS");
}
async fn historical_instance(pool: &PgPool, h: &Harness) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM agent_instances WHERE project_id=$1 AND id<>$2 ORDER BY id LIMIT 1",
    )
    .bind(h.fixture.consuming_project)
    .bind(h.command.instance_id().as_uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn missing_fresh_product_outbox_rolls_back_gate_command_and_receipt(pool: PgPool) {
    let h = Harness::new(&pool).await;
    let before = h.rows().await;
    let mut tx = begin_actor_transaction(&h.app, &h.actor).await.unwrap();
    sqlx::query("SELECT * FROM write_qualified_instance_activation($1,$2,$3,$4,$5,$6,$7)")
        .bind(h.command.activation_id().as_uuid())
        .bind(h.command.command_key().as_bytes().as_slice())
        .bind(h.command.instance_id().as_uuid())
        .bind(h.command.expected_revision_id().as_uuid())
        .bind(1_i64)
        .bind(&h.config_wire)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *tx)
        .await
        .unwrap();
    // Deliberate privileged test corruption within this uncommitted transaction;
    // no production grant, constraint disabling or fake provider evidence.
    sqlx::query("RESET ROLE").execute(&mut *tx).await.unwrap();
    sqlx::query("DELETE FROM product_event_outbox WHERE event_id=(SELECT event_id FROM agent_instance_execution_activations WHERE id=$1)").bind(h.command.activation_id().as_uuid()).execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
    assert_eq!(h.rows().await, before);
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_instances SET run_gate_open=false,version=version+1 WHERE id=$1")
        .bind(h.command.instance_id().as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let closed = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::Closed)
    ));
    let error = sqlx::query("UPDATE agent_instances SET run_gate_open=true,version=2 WHERE id=$1")
        .bind(h.command.instance_id().as_uuid())
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
    assert_eq!(h.rows().await, closed);
}
