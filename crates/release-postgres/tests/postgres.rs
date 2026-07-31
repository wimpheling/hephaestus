//! Opt-in real-PostgreSQL reusable release and isolated instance coverage.

use agent_config::parse;
use authz_postgres::PostgresMelangeAuthorizer;
use event_postgres::ReleaseOutboxPublisher;
use forge_domain::{GitRef, ProjectId, RepositoryId};
use identity_domain::{AuthenticatedIdentity, OrganizationId, RequestId, UserId};
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, ArtifactKind,
    ArtifactPath, BuildRequestId, ContentHash, InstanceName, NetworkAccess, ParameterName,
    ParameterValue, RefSelector, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion, RuntimePolicy, TriggerPolicy,
};
use release_postgres::{
    BeginUpdateHook, CompleteBuild, CreateAttachment, CreateInstanceUpdate, ImportAgent,
    RecoverInstanceUpdate, ReleaseArtifactInput, ReleaseService, RemoveAttachment, ReviseInstance,
    SetAttachmentEnabled, UpdateDecision, UpdateHookResult, UpdateRecoveryAction,
    UpdateRecoveryDecision,
};
use runtime_types::RunId;
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use uuid::Uuid;

struct Fixture {
    actor: UserId,
    first_project: ProjectId,
    first_repository: RepositoryId,
    first_aux_repository: RepositoryId,
    second_project: ProjectId,
    second_repository: RepositoryId,
    build: BuildRequestId,
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn publishes_once_and_imports_isolated_instances_with_exact_attachments() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let fixture = seed(&pool).await;
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let artifact = ReleaseArtifactInput {
        id: ReleaseArtifactId::new(),
        path: ArtifactPath::parse("bin/reviewer").expect("artifact path should validate"),
        kind: ArtifactKind::Executable,
        mode: 0o555,
        content_hash: ContentHash::digest(b"built-reviewer-v1"),
        size_bytes: 17,
        media_type: String::from("application/octet-stream"),
        storage_key: Uuid::new_v4(),
    };
    let completed = service
        .complete_build(CompleteBuild {
            command_key: key("complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("v1.0.0").expect("version should validate"),
            release_agent_id,
            artifacts: vec![artifact],
        })
        .await
        .expect("complete imported build");
    assert_eq!(completed, release_id);
    let repeated = service
        .complete_build(CompleteBuild {
            command_key: key("complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("v1.0.0").expect("version should validate"),
            release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("ignored-on-retry")
                    .expect("artifact path should validate"),
                kind: ArtifactKind::File,
                mode: 0o444,
                content_hash: ContentHash::digest(b"ignored"),
                size_bytes: 7,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("duplicate completion should return durable release");
    assert_eq!(repeated, release_id);

    let actor = identity(fixture.actor);
    service
        .publish(&actor, key("publish", release_id.as_uuid()), release_id)
        .await
        .expect("source maintainer should publish");
    let immutable = sqlx::query("UPDATE releases SET source_commit = $2 WHERE id = $1")
        .bind(release_id.as_uuid())
        .bind("b".repeat(40))
        .execute(&pool)
        .await;
    assert!(immutable.is_err(), "published provenance must be immutable");

    let first_instance = AgentInstanceId::new();
    let first_revision = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-first", first_instance.as_uuid()),
                instance_id: first_instance,
                revision_id: first_revision,
                project_id: fixture.first_project,
                release_agent_id,
                name: InstanceName::parse("reviewer").expect("name should validate"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("first project should import");
    let second_instance = AgentInstanceId::new();
    let second_revision = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-second", second_instance.as_uuid()),
                instance_id: second_instance,
                revision_id: second_revision,
                project_id: fixture.second_project,
                release_agent_id,
                name: InstanceName::parse("reviewer").expect("name should validate"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: RuntimePolicy {
                    vcpus: 1,
                    memory_mib: 512,
                    network: NetworkAccess::Disabled,
                },
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("second project should independently import");

    let instances: Vec<(Uuid, Uuid, Uuid, bool)> = sqlx::query_as(
        "SELECT instance.id, instance.project_id, instance.state_volume_id,
                revision.runnable
         FROM agent_instances AS instance
         JOIN agent_instance_revisions AS revision
           ON revision.id = instance.active_revision_id
         WHERE instance.id = ANY($1)
         ORDER BY instance.id",
    )
    .bind(vec![first_instance.as_uuid(), second_instance.as_uuid()])
    .fetch_all(&pool)
    .await
    .expect("stored isolated instances");
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].1, instances[1].1);
    assert_ne!(instances[0].2, instances[1].2);
    assert!(
        instances.iter().all(|row| !row.3),
        "required symbolic secret slot should be visibly unrunnable"
    );
    let consumer_runtime_override_columns: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'agent_instance_revisions'
           AND column_name = ANY($1)",
    )
    .bind(vec![
        "command",
        "arguments",
        "working_directory",
        "root_image_digest",
        "mounts",
        "requires_state",
    ])
    .fetch_one(&pool)
    .await
    .expect("inspect consumer revision columns");
    assert_eq!(
        consumer_runtime_override_columns, 0,
        "consumer revisions must reference immutable release-owned runtime fields"
    );
    let revised_first = AgentInstanceRevisionId::new();
    service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("revise-first", revised_first.as_uuid()),
                instance_id: first_instance,
                expected_revision_id: first_revision,
                new_revision_id: revised_first,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("parameter change should create an immutable revision");
    let history: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, parameters ->> 'severity'
         FROM agent_instance_revisions
         WHERE instance_id = $1 ORDER BY created_at, id",
    )
    .bind(first_instance.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("immutable revision history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].1, "warning");
    assert_eq!(history[1].1, "error");
    let stale_revision = service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("stale-revise", Uuid::new_v4()),
                instance_id: first_instance,
                expected_revision_id: first_revision,
                new_revision_id: AgentInstanceRevisionId::new(),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        stale_revision,
        Err(release_postgres::ReleaseServiceError::StaleInstanceRevision)
    ));
    let broadened_revision_id = AgentInstanceRevisionId::new();
    let broadened = service
        .revise_instance(
            &actor,
            ReviseInstance {
                command_key: key("broaden-policy", broadened_revision_id.as_uuid()),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                new_revision_id: broadened_revision_id,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: RuntimePolicy {
                    vcpus: 5,
                    memory_mib: 1024,
                    network: NetworkAccess::Egress,
                },
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        broadened,
        Err(release_postgres::ReleaseServiceError::Domain(
            release_domain::ReleaseValueError::PolicyBroadening
        ))
    ));
    let broadened_persisted: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM agent_instance_revisions WHERE id = $1
         )",
    )
    .bind(broadened_revision_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("inspect rejected policy revision");
    assert!(!broadened_persisted);
    let unsupported_update_id = AgentUpdateId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("unsupported-update", unsupported_update_id.as_uuid()),
                update_id: unsupported_update_id,
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: release_agent_id,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("unsupported candidate should remain visible");
    let unsupported: (String, bool, bool) = sqlx::query_as(
        "SELECT update.state,
                EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(update.diagnostics) AS item
                    WHERE item->>'code' = 'stateful_update_hook_missing'
                ),
                instance.run_gate_open
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(unsupported_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("unsupported update diagnostics");
    assert_eq!(unsupported.0, "rejected");
    assert!(unsupported.1);
    assert!(unsupported.2);

    let fork_release_agent = seed_fork_release(
        &pool,
        release_id,
        release_agent_id,
        fixture.second_repository,
    )
    .await;
    let fork_update = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("fork-update", Uuid::new_v4()),
                update_id: AgentUpdateId::new(),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: fork_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        fork_update,
        Err(release_postgres::ReleaseServiceError::AgentFamilyMismatch)
    ));

    let update_release_agent = seed_update_release(&pool, release_id, release_agent_id).await;
    let update_release_id: Uuid =
        sqlx::query_scalar("SELECT release_id FROM release_agents WHERE id = $1")
            .bind(update_release_agent.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("candidate release");
    let deferred_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-before-update", deferred_attachment.as_uuid()),
                attachment_id: deferred_attachment,
                instance_id: first_instance,
                repository_id: fixture.first_aux_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("pre-update attachment");
    let deferred_receive = Uuid::new_v4();
    let deferred_commit = "e".repeat(40);
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'deferred-test', 'accepted', now())",
    )
    .bind(deferred_receive)
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&pool)
    .await
    .expect("seed exact target receive");
    let prior_request_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, receive_id,
          run_id, command_id, instance_id, instance_revision_id,
          release_id, release_agent_id, attachment_id, request_kind,
          platform_policy_version, requires_state, dispatch_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $6, $7, $8,
                 $9, $10, $11, 'instance_normal', 'platform/v2', true,
                 'pending')",
    )
    .bind(prior_request_id)
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(first_instance.as_uuid())
    .bind(revised_first.as_uuid())
    .bind(release_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .execute(&pool)
    .await
    .expect("seed prior exact-revision request");
    let update_id = AgentUpdateId::new();
    let update_candidate_revision = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("valid-update", update_id.as_uuid()),
                update_id,
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: update_candidate_revision,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await
        .expect("valid stateful candidate should close the gate");
    let concurrent_update = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("concurrent-update", Uuid::new_v4()),
                update_id: AgentUpdateId::new(),
                instance_id: first_instance,
                expected_revision_id: revised_first,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v2"),
            },
        )
        .await;
    assert!(matches!(
        concurrent_update,
        Err(release_postgres::ReleaseServiceError::ConcurrentUpdate)
    ));
    let deferred_trigger_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO deferred_agent_triggers
         (id, instance_id, attachment_id, repository_id, target_ref,
          target_commit, source_id)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6)",
    )
    .bind(deferred_trigger_id)
    .bind(first_instance.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .execute(&pool)
    .await
    .expect("defer trigger behind closed gate");
    let draining: (String, String, bool) = sqlx::query_as(
        "SELECT update.state, instance.state, instance.run_gate_open
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("draining update");
    assert_eq!(
        draining,
        (
            String::from("draining"),
            String::from("update_draining"),
            false
        )
    );
    let drain_probe = service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("drain-probe", update_id.as_uuid()),
                update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await;
    assert!(matches!(
        drain_probe,
        Err(release_postgres::ReleaseServiceError::UpdateDrainPending)
    ));
    sqlx::query(
        "UPDATE run_requests SET dispatch_state = 'dispatched'
         WHERE id = $1",
    )
    .bind(prior_request_id)
    .execute(&pool)
    .await
    .expect("drain pre-gate request");
    let volume_id: Uuid =
        sqlx::query_scalar("SELECT state_volume_id FROM agent_instances WHERE id = $1")
            .bind(first_instance.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("instance state volume");
    sqlx::query(
        "UPDATE agent_instance_state_volumes
         SET state = 'ready', host_id = 'test-host',
             host_path = $2, filesystem_uuid = $3
         WHERE id = $1",
    )
    .bind(volume_id)
    .bind(format!("/var/lib/hephaestus-test/{volume_id}"))
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("allocate update volume fixture");
    let hook_run_id = RunId::new();
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("begin-hook", update_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
        .expect("drained update should acquire the fenced volume");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("begin-hook", update_id.as_uuid()),
                update_id,
                hook_run_id,
            },
        )
        .await
        .expect("duplicate hook admission should resolve idempotently");
    let update_start_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_id = $1 AND subject = 'hephaestus.run.start'",
    )
    .bind(hook_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("one exact update start command");
    assert_eq!(update_start_count, 1);
    let update_start: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM outbox
         WHERE aggregate_id = $1 AND subject = 'hephaestus.run.start'",
    )
    .bind(hook_run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact update start command");
    assert_eq!(update_start["kind"], "update");
    assert_eq!(update_start["attachment_id"], serde_json::Value::Null);
    assert_eq!(update_start["run_id"], hook_run_id.to_string());
    assert_eq!(update_start["requires_state"], true);
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'succeeded', exit_code = 0,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(hook_run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("persist cleaned successful update run");
    let activated = service
        .reconcile_update_run(hook_run_id)
        .await
        .expect("reconcile and activate exact committed candidate");
    assert_eq!(activated, UpdateDecision::Activated);
    let active_after_update: (Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active candidate");
    assert_eq!(
        active_after_update,
        (
            update_candidate_revision.as_uuid(),
            String::from("active"),
            true
        )
    );
    let completed_update: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM application_events
             WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
               AND event_type = 'agent_instance.changed'
               AND safe_state = 'active'
         )",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical completed update event");
    assert!(completed_update);
    let identity_preserved: (Uuid, bool) = sqlx::query_as(
        "SELECT instance.state_volume_id,
                EXISTS(
                    SELECT 1 FROM agent_attachments
                    WHERE id = $2 AND instance_id = instance.id
                )
         FROM agent_instances AS instance WHERE instance.id = $1",
    )
    .bind(first_instance.as_uuid())
    .bind(deferred_attachment.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("instance identity after update");
    assert_eq!(identity_preserved, (volume_id, true));
    let volume_ready: String =
        sqlx::query_scalar("SELECT state FROM agent_instance_state_volumes WHERE id = $1")
            .bind(volume_id)
            .fetch_one(&pool)
            .await
            .expect("released update volume");
    assert_eq!(volume_ready, "ready");
    let materialized_deferred: (String, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT deferred.state, request.instance_revision_id,
                request.release_agent_id, request.commit_sha
         FROM deferred_agent_triggers AS deferred
         JOIN run_requests AS request ON request.id = deferred.run_request_id
         WHERE deferred.id = $1",
    )
    .bind(deferred_trigger_id)
    .fetch_one(&pool)
    .await
    .expect("materialized deferred trigger");
    assert_eq!(
        materialized_deferred,
        (
            String::from("materialized"),
            update_candidate_revision.as_uuid(),
            update_release_agent.as_uuid(),
            deferred_commit,
        ),
        "deferred work must bind only the revision active after gate reopen"
    );
    let exact_revision_requests: Vec<(Uuid, Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT request.instance_revision_id, request.release_id,
                revision.parameter_hash
         FROM run_requests AS request
         JOIN agent_instance_revisions AS revision
           ON revision.id = request.instance_revision_id
         WHERE request.receive_id = $1",
    )
    .bind(deferred_receive)
    .fetch_all(&pool)
    .await
    .expect("exact requests across active revisions");
    assert_eq!(exact_revision_requests.len(), 2);
    let prior_request = exact_revision_requests
        .iter()
        .find(|request| request.0 == revised_first.as_uuid())
        .expect("prior revision request");
    let candidate_request = exact_revision_requests
        .iter()
        .find(|request| request.0 == update_candidate_revision.as_uuid())
        .expect("candidate revision request");
    assert_eq!(prior_request.1, release_id.as_uuid());
    assert_ne!(prior_request.1, candidate_request.1);
    assert_ne!(prior_request.2, candidate_request.2);
    sqlx::query(
        "UPDATE run_requests
         SET dispatch_state = 'dispatched'
         WHERE id = (
             SELECT run_request_id
             FROM deferred_agent_triggers WHERE id = $1
         )",
    )
    .bind(deferred_trigger_id)
    .execute(&pool)
    .await
    .expect("simulate deferred request dispatch");

    let rejected_update_id = AgentUpdateId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("agent-rejected-update", rejected_update_id.as_uuid()),
                update_id: rejected_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: AgentInstanceRevisionId::new(),
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("agent-rejected candidate");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("agent-rejected-hook", rejected_update_id.as_uuid()),
                update_id: rejected_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("agent-rejected hook");
    assert_eq!(
        service
            .record_update_hook_result(rejected_update_id, UpdateHookResult::Rejected(23))
            .await
            .expect("explicit agent rollback result"),
        UpdateDecision::AgentRejected
    );
    let agent_rejected: (Uuid, String, bool, i32) = sqlx::query_as(
        "SELECT instance.active_revision_id, instance.state,
                instance.run_gate_open, update.hook_exit_code
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(rejected_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("agent rejection state");
    assert_eq!(
        agent_rejected,
        (
            update_candidate_revision.as_uuid(),
            String::from("update_rejected"),
            true,
            23,
        )
    );
    let rejected_event: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM application_events
             WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
               AND event_type = 'agent_instance.changed'
               AND safe_state = 'rejected'
         )",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical update rejection event");
    assert!(rejected_event);

    let retry_update_id = AgentUpdateId::new();
    let retry_candidate = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("retry-update", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: retry_candidate,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("second update candidate");
    let retry_first_run = RunId::new();
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("retry-first-hook", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                hook_run_id: retry_first_run,
            },
        )
        .await
        .expect("first uncertain attempt");
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'failed', exit_signal = 9,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(retry_first_run.as_uuid())
    .execute(&pool)
    .await
    .expect("persist signal-terminated update run");
    assert_eq!(
        service
            .reconcile_update_run(retry_first_run)
            .await
            .expect("signal failure pauses uncertain update"),
        UpdateDecision::CompatibilityUnknown
    );
    let uncertain_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
           AND event_type = 'agent_instance.changed'
           AND safe_state = 'paused'",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical uncertain update events");
    assert!(uncertain_events >= 2);
    let retry_command_key = key("retry-recovery", retry_update_id.as_uuid());
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: retry_command_key,
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RetryHook,
                },
            )
            .await
            .expect("operator-authorized retry"),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: retry_command_key,
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RetryHook,
                },
            )
            .await
            .expect("retry recovery command is idempotent"),
        UpdateRecoveryDecision::HookRetryScheduled
    );
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("retry-second-hook", retry_update_id.as_uuid()),
                update_id: retry_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("retry uses the same update identity");
    service
        .record_update_hook_result(retry_update_id, UpdateHookResult::Uncertain)
        .await
        .expect("second uncertain attempt");
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: key("reject-recovery", retry_update_id.as_uuid()),
                    update_id: retry_update_id,
                    action: UpdateRecoveryAction::RejectCandidate,
                },
            )
            .await
            .expect("operator rejects uncertain candidate"),
        UpdateRecoveryDecision::CandidateRejected
    );
    let rejected_recovery: (Uuid, String, bool, String) = sqlx::query_as(
        "SELECT instance.active_revision_id, instance.state,
                instance.run_gate_open, update.final_decision
         FROM agent_updates AS update
         JOIN agent_instances AS instance ON instance.id = update.instance_id
         WHERE update.id = $1",
    )
    .bind(retry_update_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("rejected recovery state");
    assert_eq!(
        rejected_recovery,
        (
            update_candidate_revision.as_uuid(),
            String::from("update_rejected"),
            true,
            String::from("recovery"),
        )
    );

    let resume_update_id = AgentUpdateId::new();
    let resume_candidate = AgentInstanceRevisionId::new();
    service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key: key("resume-update", resume_update_id.as_uuid()),
                update_id: resume_update_id,
                instance_id: first_instance,
                expected_revision_id: update_candidate_revision,
                candidate_revision_id: resume_candidate,
                candidate_release_agent_id: update_release_agent,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v3"),
            },
        )
        .await
        .expect("activation-recovery candidate");
    service
        .begin_update_hook(
            &actor,
            BeginUpdateHook {
                command_key: key("resume-hook", resume_update_id.as_uuid()),
                update_id: resume_update_id,
                hook_run_id: RunId::new(),
            },
        )
        .await
        .expect("activation-recovery hook");
    service
        .record_update_hook_result(resume_update_id, UpdateHookResult::Committed)
        .await
        .expect("durable hook commit");
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("revocation after the hook commit point");
    sqlx::query("UPDATE agent_instances SET state = 'recovering' WHERE id = $1")
        .bind(first_instance.as_uuid())
        .execute(&pool)
        .await
        .expect("simulate activation CAS anomaly");
    assert_eq!(
        service
            .activate_committed_update(resume_update_id)
            .await
            .expect("activation anomaly becomes recovery"),
        UpdateDecision::ActivationRecovery
    );
    assert_eq!(
        service
            .recover_update(
                &actor,
                RecoverInstanceUpdate {
                    command_key: key("resume-recovery", resume_update_id.as_uuid()),
                    update_id: resume_update_id,
                    action: UpdateRecoveryAction::ResumeActivation,
                },
            )
            .await
            .expect("operator resumes durable activation"),
        UpdateRecoveryDecision::CandidateActivated
    );
    let resumed: (Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("resumed candidate");
    assert_eq!(
        resumed,
        (resume_candidate.as_uuid(), String::from("active"), true)
    );

    let first_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
                instance_id: first_instance,
                repository_id: fixture.first_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("same-project attachment should succeed");
    let historical_run = RunId::new();
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, run_kind, state, outcome,
          exit_code, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'normal', 'cleaned_up',
                 'succeeded', 0, true, now(), now())",
    )
    .bind(historical_run.as_uuid())
    .bind(Uuid::new_v4())
    .bind(first_instance.as_uuid())
    .bind(resume_candidate.as_uuid())
    .bind(update_release_id)
    .bind(update_release_agent.as_uuid())
    .bind(first_attachment.as_uuid())
    .execute(&pool)
    .await
    .expect("historical normal run");
    let second_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-second", second_attachment.as_uuid()),
                attachment_id: second_attachment,
                instance_id: second_instance,
                repository_id: fixture.second_repository,
                ref_selector: RefSelector::parse("refs/heads/release/*")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::PushAndManual,
            },
        )
        .await
        .expect("second same-project attachment should succeed");
    let attachment_isolation: (i64, i64) = sqlx::query_as(
        "SELECT count(DISTINCT attachment.id)::bigint,
                count(DISTINCT instance.state_volume_id)::bigint
         FROM agent_attachments AS attachment
         JOIN agent_instances AS instance ON instance.id = attachment.instance_id
         WHERE attachment.instance_id = $1",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("shared instance state across attachments");
    assert_eq!(
        attachment_isolation,
        (2, 1),
        "two attachments of one instance must share its one state volume"
    );
    let cross_project_attachment = AgentAttachmentId::new();
    let cross_project = service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-cross", Uuid::new_v4()),
                attachment_id: cross_project_attachment,
                instance_id: first_instance,
                repository_id: fixture.second_repository,
                ref_selector: RefSelector::parse("refs/heads/main")
                    .expect("selector should validate"),
                trigger_policy: TriggerPolicy::Manual,
            },
        )
        .await;
    assert!(cross_project.is_err());
    let rolled_back_messages: i64 =
        sqlx::query_scalar("SELECT count(*) FROM outbox WHERE aggregate_id = $1")
            .bind(cross_project_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("rolled-back command has no message");
    assert_eq!(rolled_back_messages, 0);
    service
        .set_attachment_enabled(
            &actor,
            SetAttachmentEnabled {
                command_key: key("disable-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
                enabled: false,
            },
        )
        .await
        .expect("authorized attachment disable");
    let disabled: bool =
        sqlx::query_scalar("SELECT NOT enabled FROM agent_attachments WHERE id = $1")
            .bind(first_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("disabled attachment");
    assert!(disabled);
    service
        .remove_attachment(
            &actor,
            RemoveAttachment {
                command_key: key("remove-first", first_attachment.as_uuid()),
                attachment_id: first_attachment,
            },
        )
        .await
        .expect("authorized attachment tombstone");
    let tombstoned: bool = sqlx::query_scalar(
        "SELECT removed_at IS NOT NULL AND NOT enabled
         FROM agent_attachments WHERE id = $1",
    )
    .bind(first_attachment.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("tombstoned attachment");
    assert!(tombstoned);
    let attachment_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'agent_instance' AND aggregate_id = $1
           AND event_type = 'agent_instance.changed'",
    )
    .bind(first_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("canonical attachment invalidations");
    assert!(attachment_events >= 3);
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("authorized release revocation");
    service
        .revoke(
            &actor,
            key("revoke-release", update_release_id),
            ReleaseId::from_uuid(update_release_id),
        )
        .await
        .expect("release revocation replay is idempotent");
    let preserved_history: (Uuid, Uuid, Uuid, Uuid, bool, String) = sqlx::query_as(
        "SELECT run.id, revision.id, release_agent.id, release.id,
                attachment.removed_at IS NOT NULL, release.state
         FROM runs AS run
         JOIN agent_instance_revisions AS revision
           ON revision.id = run.instance_revision_id
          AND revision.instance_id = run.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = run.release_agent_id
          AND release_agent.release_id = run.release_id
         JOIN releases AS release ON release.id = run.release_id
         JOIN agent_attachments AS attachment
           ON attachment.id = run.attachment_id
          AND attachment.instance_id = run.instance_id
         WHERE run.id = $1",
    )
    .bind(historical_run.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("historical foreign-key targets after tombstones");
    assert_eq!(
        preserved_history,
        (
            historical_run.as_uuid(),
            resume_candidate.as_uuid(),
            update_release_agent.as_uuid(),
            update_release_id,
            true,
            String::from("revoked"),
        )
    );
    let revocation_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_type = 'release' AND aggregate_id = $1
           AND event_type = 'release.changed' AND safe_state = 'revoked'",
    )
    .bind(update_release_id)
    .fetch_one(&pool)
    .await
    .expect("one release revocation event");
    assert_eq!(revocation_events, 1);

    let artifact_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_artifacts WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("artifact count");
    assert_eq!(artifact_count, 1);
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events
         WHERE aggregate_id IN ($1, $2, $3)",
    )
    .bind(release_id.as_uuid())
    .bind(first_instance.as_uuid())
    .bind(second_instance.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable event count");
    assert!(event_count >= 4);
}

#[tokio::test]
#[serial]
async fn release_outbox_retry_is_deduplicated_by_jetstream() {
    let (Ok(nats_url), Some(pool)) = (std::env::var("HEPHAESTUS_NATS_TEST_URL"), pool().await)
    else {
        return;
    };
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'release' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate release outbox fixture");
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type, payload, occurred_at)
         VALUES
         ($1, 'release', $2, 'hephaestus.instance.run.requested.v1',
          'instance.run.requested.v1', '{}', now()),
         ($3, 'release', $4, 'hephaestus.run.start',
          'run.start.v1', '{}', now())",
    )
    .bind(first_id)
    .bind(Uuid::new_v4())
    .bind(second_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert release outbox fixture");

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_RELEASE_TEST_{}", first_id.simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![String::from("hephaestus.>")],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("isolated release stream");
    let publisher = ReleaseOutboxPublisher::new(context.clone(), pool.clone());
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("first publication"),
        2
    );
    assert_eq!(stream.info().await.expect("stream state").state.messages, 2);
    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id IN ($1, $2)")
        .bind(first_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(10)
            .await
            .expect("retry publication"),
        2
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        2
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated stream");
    sqlx::query("DELETE FROM outbox WHERE id IN ($1, $2)")
        .bind(first_id)
        .bind(second_id)
        .execute(&pool)
        .await
        .expect("clean release outbox fixture");
}

const fn selected_policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 2,
        memory_mib: 1024,
        network: NetworkAccess::BrokerOnly,
    }
}

const fn platform_policy() -> RuntimePolicy {
    RuntimePolicy {
        vcpus: 8,
        memory_mib: 8192,
        network: NetworkAccess::Egress,
    }
}

fn key(operation: &str, id: Uuid) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(operation, &[id.as_bytes()])
}

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.release.test",
        format!("release-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect PostgreSQL"),
    )
}

#[allow(clippy::too_many_lines)]
async fn seed(pool: &PgPool) -> Fixture {
    let actor = UserId::new();
    let organization = OrganizationId::new();
    let source_project = ProjectId::new();
    let source_repository = RepositoryId::new();
    let first_project = ProjectId::new();
    let first_repository = RepositoryId::new();
    let first_aux_repository = RepositoryId::new();
    let second_project = ProjectId::new();
    let second_repository = RepositoryId::new();
    let receive_id = Uuid::new_v4();
    let build = BuildRequestId::new();
    let commit = "a".repeat(40);
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(actor.as_uuid())
        .bind(format!("release-actor-{actor}"))
        .execute(pool)
        .await
        .expect("seed actor");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization.as_uuid())
        .bind(format!("release-org-{organization}"))
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization.as_uuid())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed member");
    for (project, name) in [
        (source_project, "source"),
        (first_project, "first"),
        (second_project, "second"),
    ] {
        sqlx::query(
            "INSERT INTO projects (id, organization_id, name)
             VALUES ($1, $2, $3)",
        )
        .bind(project.as_uuid())
        .bind(organization.as_uuid())
        .bind(format!("{name}-{project}"))
        .execute(pool)
        .await
        .expect("seed project");
        sqlx::query(
            "INSERT INTO project_maintainers (project_id, user_id)
             VALUES ($1, $2)",
        )
        .bind(project.as_uuid())
        .bind(actor.as_uuid())
        .execute(pool)
        .await
        .expect("seed project maintainer");
    }
    for (repository, project, name) in [
        (source_repository, source_project, "source"),
        (first_repository, first_project, "first"),
        (first_aux_repository, first_project, "first-aux"),
        (second_repository, second_project, "second"),
    ] {
        sqlx::query(
            "INSERT INTO repositories
             (id, project_id, name, default_branch, is_public)
             VALUES ($1, $2, $3, 'refs/heads/main', false)",
        )
        .bind(repository.as_uuid())
        .bind(project.as_uuid())
        .bind(format!("{name}-{repository}"))
        .execute(pool)
        .await
        .expect("seed repository");
    }
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'release-test', 'accepted', now())",
    )
    .bind(receive_id)
    .bind(source_repository.as_uuid())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed receive");
    let source = reusable_config();
    let parsed = parse(source.as_bytes());
    let config = parsed.config.expect("fixture configuration should parse");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash,
          normalized_config_hash, schema_version, status, config, diagnostics)
         VALUES ($1, $2, $3, $4, $5, $6, 2, 'valid', $7, '[]')",
    )
    .bind(Uuid::new_v4())
    .bind(source_repository.as_uuid())
    .bind(receive_id)
    .bind(&commit)
    .bind(parsed.hash.as_str())
    .bind(
        parsed
            .normalized_hash
            .expect("fixture normalized hash")
            .as_str(),
    )
    .bind(serde_json::to_value(config).expect("serialize fixture config"))
    .execute(pool)
    .await
    .expect("seed reusable configuration");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'importing', $7)",
    )
    .bind(build.as_uuid())
    .bind(source_repository.as_uuid())
    .bind(commit)
    .bind(
        GitRef::parse("refs/heads/main")
            .expect("ref should parse")
            .as_str(),
    )
    .bind(receive_id)
    .bind([9_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed importing build");
    Fixture {
        actor,
        first_project,
        first_repository,
        first_aux_repository,
        second_project,
        second_repository,
        build,
    }
}

async fn seed_update_release(
    pool: &PgPool,
    current_release_id: ReleaseId,
    current_release_agent_id: ReleaseAgentId,
) -> ReleaseAgentId {
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         SELECT $1, repository_id, 'v2.0.0', source_commit, source_ref,
                build_request_id, build_definition_hash, configuration,
                configuration_hash, manifest_hash, 'published', now()
         FROM releases WHERE id = $2",
    )
    .bind(release_id.as_uuid())
    .bind(current_release_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed update release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, $2, family_id, agent_key, display_name,
                runtime_contract, $3, parameter_schema, '[]',
                requires_state, $4
         FROM release_agents WHERE id = $5",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind([42_u8; 32].as_slice())
    .bind(json!({
        "command": "bin/update",
        "arguments": [],
        "timeout_seconds": 60
    }))
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed update release agent");
    release_agent_id
}

async fn seed_fork_release(
    pool: &PgPool,
    current_release_id: ReleaseId,
    current_release_agent_id: ReleaseAgentId,
    fork_repository_id: RepositoryId,
) -> ReleaseAgentId {
    let build_id = BuildRequestId::new();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let family_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id.as_uuid())
    .bind(fork_repository_id.as_uuid())
    .bind("d".repeat(40))
    .bind([71_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed fork build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         SELECT $1, $2, 'v1.0.0-fork', $3, source_ref,
                $4, $5, configuration, configuration_hash,
                manifest_hash, 'published', now()
         FROM releases WHERE id = $6",
    )
    .bind(release_id.as_uuid())
    .bind(fork_repository_id.as_uuid())
    .bind("d".repeat(40))
    .bind(build_id.as_uuid())
    .bind([71_u8; 32].as_slice())
    .bind(current_release_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed fork release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         SELECT $1, $2, agent_key
         FROM release_agents WHERE id = $3",
    )
    .bind(family_id)
    .bind(fork_repository_id.as_uuid())
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed fork family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state, update_hook)
         SELECT $1, $2, $3, agent_key, display_name,
                runtime_contract, $4, parameter_schema, secret_slot_schema,
                requires_state, update_hook
         FROM release_agents WHERE id = $5",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind(family_id)
    .bind([72_u8; 32].as_slice())
    .bind(current_release_agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed fork release agent");
    release_agent_id
}

fn reusable_config() -> String {
    let digest = "a".repeat(64);
    format!(
        r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
command = "/bin/build"
working_directory = "/source"
root_image = "build@sha256:{digest}"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 2
memory_mib = 1024
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
[guest]
command = "bin/reviewer"
arguments = ["--json"]
working_directory = "bin"
[resources]
vcpus = 4
memory_mib = 2048
[root_image]
reference = "runtime@sha256:{digest}"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[network]
profile = "egress"
[triggers]
push = false
[[parameters]]
name = "severity"
type = "enum"
values = ["warning", "error"]
required = true
[[secret_slots]]
key = "model"
purpose = "Call a configured model"
required = true
delivery_modes = ["brokered"]
phases = ["normal"]
destinations = ["api.example.test"]
"#
    )
}
