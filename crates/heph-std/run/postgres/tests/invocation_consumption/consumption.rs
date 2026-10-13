use identity_domain::RequestId;
use release_service::InstanceExecutionService;
use run_orchestrator::{RunRepository, RunRuntimeCatalog};
use sqlx::PgPool;

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn actual_preexisting_saved_invocation_is_consumed_without_run_creation_or_git(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let result = h.consume().await.unwrap();
    assert!(!result.created);
    assert_eq!(result.run.id, h.start.run_id);
    assert_eq!(result.run.state_version, 0);
    assert_eq!(result.run.vm_id.as_deref(), Some(h.vm.0.as_str()));
    assert_eq!(result.run.state, run_domain::RunState::Queued);
    let exact: bool = sqlx::query_scalar("SELECT c.start_payload=i.payload AND i.processed_at IS NOT NULL AND e.actor_type='system' AND e.actor_id IS NULL AND e.request_id IS NULL AND inspect_qualified_invocation117(c.run_id) AND NOT named_volume_dispatch_supported(q.instance_id) FROM run_invocation_consumptions c JOIN command_inbox i ON i.command_id=c.start_command_id JOIN application_events e ON e.id=c.event_id JOIN agent_instance_execution_invocations q ON q.id=c.invocation_id WHERE c.run_id=$1")
        .bind(h.start.run_id.as_uuid()).fetch_one(&h.producer.activation.worker).await.unwrap();
    assert!(
        exact,
        "worker system provenance is separate from original caller authority"
    );
    let input = h.repository().load_runtime(&result.run).await.unwrap();
    assert_eq!(input.parameters, serde_json::json!({"greeting":"saved-é"}));
    assert_eq!(input.artifacts.len(), 1);
    assert_eq!(input.artifacts[0].path, "bin/sqlite");
    assert_eq!(input.artifacts[0].content_hash, [37_u8; 32]);
    assert_eq!(input.artifacts[0].size_bytes, 7);
    assert!(input.repository_id.is_none() && input.git_ref.is_none() && input.commit_sha.is_none());
    assert!(
        input.update_id.is_none()
            && input.previous_revision_id.is_none()
            && input.previous_release_id.is_none()
    );
    assert!(
        input.previous_parameters.is_none()
            && input.previous_artifacts.is_empty()
            && input.mailbox_event.is_none()
    );
    assert_eq!(
        h.rows().await["producer"]["runs"],
        before["producer"]["runs"]
    );
    assert_eq!(
        h.rows().await["producer"]["outbox"],
        before["producer"]["outbox"]
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn same_input_concurrent_and_new_request_replays_keep_original_receipt(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let (first, second) = tokio::join!(h.consume(), h.consume());
    assert_eq!(first.unwrap().run, second.unwrap().run);
    let before = h.rows().await;
    let mut fresh = h.producer.activation.actor.clone();
    fresh.request_id = RequestId::new();
    h.producer
        .service()
        .invoke_instance(&fresh, h.producer.command)
        .await
        .unwrap();
    assert!(!h.consume().await.unwrap().created);
    assert_eq!(h.rows().await, before);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_invocation_consumptions WHERE run_id=$1")
            .bind(h.start.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn missing_producer_and_full_start_correlation_deny_without_adoption(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let mut variants = Vec::new();
    let mut altered = h.start.clone();
    altered.command_id = runtime_types::CommandId::new();
    variants.push(altered);
    let mut altered = h.start.clone();
    altered.release_id = runtime_types::ReleaseId::new();
    variants.push(altered);
    let mut altered = h.start.clone();
    altered.requires_state = !altered.requires_state;
    variants.push(altered);
    let mut altered = h.start.clone();
    altered.instance_revision_id = runtime_types::AgentInstanceRevisionId::new();
    variants.push(altered);
    for altered in variants {
        assert!(
            h.repository()
                .create_run_with_vm_plan(&altered, &h.scope, &h.vm)
                .await
                .is_err()
        );
        assert_eq!(h.rows().await, before);
    }
    let mut absent = h.start.clone();
    absent.run_id = runtime_types::RunId::new();
    assert!(
        h.repository()
            .create_run_with_vm_plan(
                &absent,
                &h.scope,
                &vm_trait::VmId(absent.run_id.to_string())
            )
            .await
            .is_err()
    );
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private117"]
async fn default_profile_and_wrong_config_or_actual_scope_do_not_consume(pool: PgPool) {
    let h = Fixture::new(&pool).await;
    let before = h.rows().await;
    let default = run_postgres::PgRunRepository::new(h.producer.activation.worker.clone());
    assert!(
        default
            .create_run_with_vm_plan(&h.start, &h.scope, &h.vm)
            .await
            .is_err()
    );
    let changed = release_service::InstanceExecutionConfiguration::new(
        h.producer.activation.config.provider_scope().clone(),
        "/different/prospective/root".to_owned(),
        h.producer.activation.config.volume_namespace(),
        h.producer.activation.config.platform_policy().clone(),
        h.producer.activation.config.platform_version().to_owned(),
    )
    .unwrap();
    assert!(
        h.repository()
            .with_qualified_invocation_configuration(changed)
            .create_run_with_vm_plan(&h.start, &h.scope, &h.vm)
            .await
            .is_err()
    );
    let wrong =
        run_domain::RunCleanupHostId::new("wrong-owner".to_owned(), h.scope.host_id().to_owned())
            .unwrap();
    assert!(
        h.repository()
            .create_run_with_vm_plan(&h.start, &wrong, &h.vm)
            .await
            .is_err()
    );
    let wrong_vm = vm_trait::VmId(uuid::Uuid::new_v4().to_string());
    assert!(
        h.repository()
            .create_run_with_vm_plan(&h.start, &h.scope, &wrong_vm)
            .await
            .is_err()
    );
    assert!(
        default.get(h.start.run_id).await.is_err(),
        "default hydration remains closed"
    );
    assert_eq!(h.rows().await, before);
}
