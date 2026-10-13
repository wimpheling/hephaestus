use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use release_domain::AgentInstanceId;
use release_service::{InstanceExecutionError, InstanceExecutionService, InvokeInstance};
use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::{Invocation, code};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn trusted_provider_configuration_drift_holds_first_and_replay(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let changed = crate::activation_fixture::configuration(
        "different-vm-owner",
        "/tmp",
        Uuid::new_v4(),
        "platform/changed",
    );
    let before = h.rows().await;
    let service = h
        .activation
        .configured(changed)
        .with_qualified_invocation_admission();
    assert!(matches!(
        service
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::ConfigurationConflict)
    ));
    assert_eq!(h.rows().await, before);
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    let committed = h.rows().await;
    assert!(matches!(
        service
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::ConfigurationConflict)
    ));
    assert_eq!(h.rows().await, committed);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn existing_historical_instance_cannot_borrow_qualified_activation(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let foreign: Uuid = sqlx::query_scalar(
        "SELECT id FROM agent_instances WHERE project_id=$1 AND id<>$2 ORDER BY id LIMIT 1",
    )
    .bind(h.activation.fixture.consuming_project)
    .bind(h.command.instance_id().as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let command = InvokeInstance::new(
        h.command.command_key(),
        h.command.invocation_id(),
        h.command.run_id(),
        h.command.start_command_id(),
        AgentInstanceId::from_uuid(foreign),
        h.command.expected_revision_id(),
    )
    .unwrap();
    let before = h.rows().await;
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, command)
            .await,
        Err(InstanceExecutionError::Unsupported)
    ));
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn deferred_missing_product_outbox_rolls_back_entire_admission(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let before = h.rows().await;
    let mut tx = h.transaction().await;
    h.write(&mut tx).await;
    // Privileged test corruption is in the SAME uncommitted transaction; no
    // production grants, constraint disabling or fabricated physical evidence.
    sqlx::query("RESET ROLE").execute(&mut *tx).await.unwrap();
    sqlx::query("DELETE FROM product_event_outbox WHERE event_id=(SELECT event_id FROM agent_instance_execution_invocations WHERE id=$1)").bind(h.command.invocation_id().as_uuid()).execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"), "{error:?}");
    assert_eq!(h.rows().await, before);
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn raw_actor_worker_and_profile_labels_cannot_manufacture_producer(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let before = h.rows().await;
    let mut tx = h.transaction().await;
    let error = sqlx::query("INSERT INTO agent_instance_execution_invocations(id) VALUES($1)")
        .bind(Uuid::new_v4())
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"));
    tx.rollback().await.unwrap();
    let error =
        sqlx::query("SELECT * FROM write_qualified_instance_invocation($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(h.command.invocation_id().as_uuid())
            .bind(h.command.command_key().as_bytes().as_slice())
            .bind(h.command.run_id().as_uuid())
            .bind(h.command.start_command_id().as_uuid())
            .bind(h.command.instance_id().as_uuid())
            .bind(h.command.expected_revision_id().as_uuid())
            .bind(&h.activation.config_wire)
            .bind(AUTHORIZATION_MODEL_VERSION)
            .execute(&h.activation.worker)
            .await
            .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("42501"));
    let mut tx = begin_actor_transaction(&h.activation.worker, &h.activation.actor)
        .await
        .unwrap();
    let error=sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,requires_state,state,state_version,created_at,updated_at,execution_profile,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,vm_plan_admission_required) VALUES($1,$2,$3,$4,$5,NULL,'invocation',$6,false,'queued',0,now(),now(),'owned_canonical',$1::text,'vm-owner-fixture','fixture-host','planned',false)")
        .bind(h.command.run_id().as_uuid()).bind(h.command.instance_id().as_uuid()).bind(h.command.expected_revision_id().as_uuid()).bind(h.activation.fixture.release).bind(h.activation.fixture.release_agent).bind(h.command.start_command_id().as_uuid()).execute(&mut *tx).await.unwrap_err();
    assert!(
        matches!(code(&error).as_deref(), Some("23000" | "42501")),
        "exact guarded admission denial, not missing schema: {error:?}"
    );
    tx.rollback().await.unwrap();
    let protected:bool=sqlx::query_scalar("SELECT (SELECT bool_and(relrowsecurity AND relforcerowsecurity) FROM pg_class WHERE oid IN ('agent_instance_execution_invocations'::regclass,'agent_instance_execution_invocation_audits'::regclass)) AND NOT has_table_privilege('hephaestus_app','agent_instance_execution_invocations','INSERT') AND NOT has_table_privilege('hephaestus_worker','agent_instance_execution_invocations','INSERT') AND NOT has_function_privilege('hephaestus_worker','write_qualified_instance_invocation(uuid,bytea,uuid,uuid,uuid,uuid,bytea,text)','EXECUTE') AND EXISTS(SELECT 1 FROM pg_proc WHERE oid='write_qualified_instance_invocation(uuid,bytea,uuid,uuid,uuid,uuid,bytea,text)'::regprocedure AND prosecdef AND proconfig @> ARRAY['search_path=pg_catalog, public, pg_temp'])").fetch_one(&pool).await.unwrap();
    assert!(
        protected,
        "forced RLS and exact actor-only protected writer"
    );
    assert_eq!(h.rows().await, before);
}
