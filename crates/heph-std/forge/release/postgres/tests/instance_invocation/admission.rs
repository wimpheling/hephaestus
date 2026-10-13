use identity_domain::RequestId;
use release_service::{InstanceExecutionError, InstanceExecutionService};
use sqlx::PgPool;

use super::fixture::Invocation;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn slotless_invocation_commits_exact_saved_graph_without_runtime_io(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let admission = h
        .service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    assert_eq!(admission.invocation_id(), h.command.invocation_id());
    assert_eq!(admission.run_id(), h.command.run_id());
    assert_eq!(admission.start_command_id(), h.command.start_command_id());
    let exact:bool=sqlx::query_scalar("SELECT q.admission_version=115 AND q.instance_version=2 AND r.run_kind='invocation' AND r.state='queued' AND r.state_version=0 AND r.attachment_id IS NULL AND r.execution_profile='owned_canonical' AND r.vm_id=r.id::text AND r.vm_binding_kind='planned' AND r.vm_provider_namespace=q.config_json->'vm'->>'namespace' AND r.vm_host_id=q.config_json->'vm'->>'host_id' AND q.parameters=v.parameters AND q.parameter_hash=v.parameter_hash AND q.effective_policy=v.effective_runtime_policy AND q.effective_policy_hash=v.effective_policy_hash AND q.platform_policy_version=v.platform_policy_version AND r.instance_revision_id=q.revision_id AND i.version=2 AND i.run_gate_open AND qualified_invocation_graph_matches(q) AND NOT named_volume_dispatch_supported(q.instance_id) AND NOT EXISTS(SELECT 1 FROM run_requests WHERE run_id=r.id) AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=r.id) AND NOT EXISTS(SELECT 1 FROM agent_updates WHERE hook_run_id=r.id) AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=r.id) FROM agent_instance_execution_invocations q JOIN runs r ON r.id=q.run_id JOIN agent_instances i ON i.id=q.instance_id JOIN agent_instance_revisions v ON v.id=q.revision_id WHERE q.id=$1")
        .bind(h.command.invocation_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        exact,
        "committed admission data is not runtime/VM or mount proof"
    );
    let payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM command_inbox WHERE command_id=$1")
            .bind(h.command.start_command_id().as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payload["kind"], "invocation");
    assert!(payload["attachment_id"].is_null());
    for absent in [
        "git_ref",
        "commit_sha",
        "repository_id",
        "parameters",
        "policy",
    ] {
        assert!(payload.get(absent).is_none());
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn same_actor_new_request_replays_original_graph_and_fresh_audits(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let first = h
        .service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    let before = h.rows().await;
    let mut retry = h.activation.actor.clone();
    retry.request_id = RequestId::new();
    assert_eq!(
        h.service()
            .invoke_instance(&retry, h.command)
            .await
            .unwrap(),
        first
    );
    assert_eq!(h.rows().await, before);
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM authorization_audit_events WHERE actor_id=$1 AND request_id=$2 AND decision='allow' AND ((permission='can_execute' AND object_type='agent_instance' AND object_id=$3) OR (permission='can_use' AND object_type='release_agent' AND object_id=$4))")
        .bind(retry.user_id.as_uuid()).bind(retry.request_id.as_uuid()).bind(h.command.instance_id().as_uuid()).bind(h.activation.fixture.release_agent).fetch_one(&pool).await.unwrap();
    assert!(
        audits >= 2,
        "fresh middleware request is audited without rewriting original lineage"
    );
    let original:bool=sqlx::query_scalar("SELECT actor_id=$2 AND request_id=$3 FROM agent_instance_execution_invocations WHERE id=$1")
        .bind(h.command.invocation_id().as_uuid()).bind(h.activation.actor.user_id.as_uuid()).bind(h.activation.actor.request_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(original);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn default_service_and_unactivated_birth_do_not_implicitly_activate(pool: PgPool) {
    let h = Invocation::unactivated(&pool).await;
    let before = h.rows().await;
    assert!(matches!(
        h.activation
            .service()
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::Unsupported)
    ));
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::Closed)
    ));
    assert_eq!(h.rows().await, before);
    h.activation
        .service()
        .activate_instance(&h.activation.actor, h.activation.command)
        .await
        .unwrap();
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
}
