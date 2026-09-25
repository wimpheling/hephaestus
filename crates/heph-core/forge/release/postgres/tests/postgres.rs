//! Opt-in real-PostgreSQL reusable release and isolated instance coverage.

use agent_config::{AgentConfig, parse, parse_repository_gateways, parse_repository_uis};
use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::{
    CapabilityBindingId, CapabilityOperation, CapabilityResource, CapabilityResourceKind,
    CapabilitySlotKey,
};
use forge_domain::{GitRef, ProjectId, RepositoryId};
use futures_util::StreamExt;
use identity_domain::{
    AuthenticatedIdentity, OrganizationId, RequestId, UserId, actor_idempotency_id,
};
use mailbox_dispatch::{
    MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchCommand, MailboxDispatchStore,
};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId,
    MailboxOperationIdentity, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, ArtifactKind,
    ArtifactPath, BuildRequestId, ContentHash, InstanceName, NetworkAccess, ParameterName,
    ParameterValue, RefSelector, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion, RuntimePolicy, TriggerPolicy, UiInstallationCallerKey,
    UiInstallationCommandIdentity, UiInstallationGenerationId, UiInstallationOperation,
    UiInstallationTarget,
};
use release_postgres::{
    ActivateUiInstallation, BeginUpdateHook, CompleteBuild, CreateAttachment, CreateInstanceUpdate,
    DisableUiInstallation, ImportAgent, InstallStaticUi, InstallStaticUiResult, InstallUi,
    RecoverInstanceUpdate, ReleaseArtifactInput, ReleaseService, RemoveAttachment,
    RemoveUiInstallation, ReviseInstance, ReviseInstanceCapabilities, RollbackUiInstallation,
    SetAttachmentEnabled, UiInstallationError, UiInstallationGenerationResult, UpdateDecision,
    UpdateHookResult, UpdateRecoveryAction, UpdateRecoveryDecision,
};
use runtime_types::RunId;
use serde_json::{Value, json};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

type UiBindingRow = (String, String, Uuid, Uuid, Uuid, String, String, String);

struct Fixture {
    actor: UserId,
    organization: OrganizationId,
    first_project: ProjectId,
    first_repository: RepositoryId,
    first_aux_repository: RepositoryId,
    second_project: ProjectId,
    second_repository: RepositoryId,
    build: BuildRequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct BrokerRuleSnapshot {
    id: Uuid,
    binding_id: Uuid,
    instance_revision_id: Uuid,
    secret_version_id: Uuid,
    destination_origin: String,
    location_kind: String,
    header_name: String,
    header_prefix: Option<String>,
    normalized_hash: Vec<u8>,
}

#[path = "postgres/mod.rs"]
pub mod support;
use support::*;

#[path = "postgres/broker_rules.rs"]
pub mod broker_rules;

#[path = "postgres/complete_build.rs"]
pub mod complete_build;

#[path = "postgres/installation.rs"]
pub mod installation;

#[path = "postgres/install_replay.rs"]
mod install_replay;

#[path = "postgres/parent_move.rs"]
mod parent_move;

#[path = "postgres/runtime_git.rs"]
mod runtime_git;

/// Shared state passed between the isolated-instance test phases.
#[path = "postgres/isolated_context.rs"]
pub mod isolated_context;
use isolated_context::{IsolatedGateContext, IsolatedPublishedContext, IsolatedUpdateContext};

/// Release publication and initial instance setup phase.
#[path = "postgres/isolated_setup.rs"]
pub mod isolated_setup;

/// Immutable revision and update-gate setup phase.
#[path = "postgres/isolated_updates.rs"]
pub mod isolated_updates;

/// Mailbox and transport gate setup phase.
#[path = "postgres/isolated_mailbox.rs"]
pub mod isolated_mailbox;

/// `JetStream` outbox retry and deduplication scenario.
#[path = "postgres/outbox_retry.rs"]
pub mod outbox_retry;

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
// The real JetStream pull streams are retained across the gate transition so
// this single integration test can prove transport redelivery and claim CAS.
#[allow(clippy::large_stack_frames)]
async fn publishes_once_and_imports_isolated_instances_with_exact_attachments() {
    let Some(context) = isolated_setup::prepare().await else {
        return;
    };
    let context = isolated_updates::prepare(context).await;
    let IsolatedGateContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        first_instance,
        second_instance,
        revised_first,
        update_release_agent,
        update_release_id,
        deferred_attachment,
        deferred_receive,
        prior_request_id,
        deferred_commit,
        update_id,
        update_candidate_revision,
        mailbox_store,
        mailbox_id,
        accepted,
        dispatch,
        nats_event_id,
        ..
    } = isolated_mailbox::prepare(context).await;
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
                brokered_rule_copies: Vec::new(),
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
    let activation_wakes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = $1 AND aggregate_id = $2 AND id <> $2",
    )
    .bind(MAILBOX_WAKE_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count activation mailbox wake");
    assert_eq!(
        activation_wakes, 1,
        "activation must re-wake eligible delivery"
    );
    let activation_wake_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM outbox
         WHERE subject = $1 AND aggregate_id = $2 AND id <> $2
         ORDER BY occurred_at DESC, id DESC LIMIT 1",
    )
    .bind(MAILBOX_WAKE_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load activation mailbox wake");
    mailbox_store
        .apply_command(
            MAILBOX_WAKE_SUBJECT,
            &MailboxDispatchCommand {
                operation_id: mailbox_domain::MailboxOperationId::from_uuid(activation_wake_id),
                event_id: accepted.event_id,
            },
        )
        .await
        .expect("apply activation mailbox wake");
    let activation_dispatches: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = $1 AND aggregate_id = $2",
    )
    .bind(MAILBOX_DISPATCH_SUBJECT)
    .bind(accepted.event_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("count activation dispatch commands");
    assert_eq!(
        activation_dispatches, 2,
        "activation must enqueue one fresh dispatch"
    );
    sqlx::query(
        "INSERT INTO git_refs
         (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, 'refs/heads/main', $2, $3)",
    )
    .bind(fixture.first_aux_repository.as_uuid())
    .bind(&deferred_commit)
    .bind(deferred_receive)
    .execute(&pool)
    .await
    .expect("seed exact update-race target ref");
    let attempts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
            .bind(accepted.event_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("count update-race attempts");
    assert_eq!(
        attempts, 0,
        "wake recovery must not create an attempt itself"
    );
    let resumed = mailbox_store
        .claim_dispatch(&dispatch)
        .await
        .expect("claim post-activation update-race dispatch")
        .expect("fresh transport identity claims the candidate once");
    assert_eq!(
        resumed.instance_revision_id, update_candidate_revision,
        "post-activation dispatch must use the active candidate revision"
    );
    sqlx::query(
        "UPDATE runs
         SET state = 'cleaned_up', outcome = 'succeeded', updated_at = now()
         WHERE id = $1",
    )
    .bind(resumed.run_id.as_uuid())
    .execute(&pool)
    .await
    .expect("finish update-race proof run");
    assert!(
        mailbox_store
            .claim_dispatch(&dispatch)
            .await
            .expect("reject duplicate update-race dispatch")
            .is_none(),
        "a stale or duplicate transport command must not create another attempt"
    );
    if let Some(nats_event_id) = nats_event_id {
        let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL").expect("NATS URL remains set");
        let nats = async_nats::connect(nats_url)
            .await
            .expect("reconnect update-race NATS");
        let jetstream = async_nats::jetstream::new(nats);
        let consumer = mailbox_dispatch::ensure_mailbox_jetstream_topology(&jetstream)
            .await
            .expect("reopen update-race NATS topology");
        let publisher = mailbox_dispatch::MailboxOutboxPublisher::new(
            jetstream,
            Arc::new(mailbox_store.clone()),
        );
        let activation_wake_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM outbox
             WHERE subject = $1 AND aggregate_id = $2 AND id <> $2
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(MAILBOX_WAKE_SUBJECT)
        .bind(nats_event_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load NATS activation wake");
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-activation NATS wake");
        let wake_published: bool =
            sqlx::query_scalar("SELECT published_at IS NOT NULL FROM outbox WHERE id = $1")
                .bind(activation_wake_id)
                .fetch_one(&pool)
                .await
                .expect("inspect published NATS wake");
        assert!(wake_published, "activation wake must be accepted by NATS");
        let mut messages = consumer.messages().await.expect("open NATS consumer");
        let wake_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive post-activation NATS wake")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand =
                serde_json::from_slice(&delivery.payload).expect("NATS activation wake command");
            if delivery.message.subject.as_str() == MAILBOX_WAKE_SUBJECT
                && command.event_id == nats_event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        mailbox_store
            .apply_command(MAILBOX_WAKE_SUBJECT, &wake_delivery.1)
            .await
            .expect("apply post-activation NATS wake");
        wake_delivery
            .0
            .double_ack()
            .await
            .expect("ack post-activation NATS wake");
        let expected_operation_id =
            MailboxOperationIdentity::dispatch(mailbox_id, nats_event_id, 1).id();
        let activation_dispatch_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM outbox
             WHERE subject = $1 AND aggregate_id = $2 AND id <> $3
             ORDER BY occurred_at DESC, id DESC LIMIT 1",
        )
        .bind(MAILBOX_DISPATCH_SUBJECT)
        .bind(nats_event_id.as_uuid())
        .bind(expected_operation_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load NATS activation dispatch");
        publisher
            .publish_pending(100)
            .await
            .expect("publish post-activation NATS dispatch");
        let dispatch_published: bool =
            sqlx::query_scalar("SELECT published_at IS NOT NULL FROM outbox WHERE id = $1")
                .bind(activation_dispatch_id)
                .fetch_one(&pool)
                .await
                .expect("inspect published NATS dispatch");
        assert!(
            dispatch_published,
            "activation dispatch must be accepted by NATS"
        );
        let dispatch_delivery = loop {
            let delivery = tokio::time::timeout(Duration::from_secs(5), messages.next())
                .await
                .expect("receive post-activation NATS dispatch")
                .expect("NATS stream item")
                .expect("valid NATS message");
            let command: MailboxDispatchCommand = serde_json::from_slice(&delivery.payload)
                .expect("NATS activation dispatch command");
            if delivery.message.subject.as_str() == MAILBOX_DISPATCH_SUBJECT
                && command.event_id == nats_event_id
            {
                break (delivery, command);
            }
            delivery
                .double_ack()
                .await
                .expect("ack unrelated NATS command");
        };
        assert_eq!(dispatch_delivery.1.operation_id, expected_operation_id);
        mailbox_store
            .apply_command(MAILBOX_DISPATCH_SUBJECT, &dispatch_delivery.1)
            .await
            .expect("apply post-activation NATS dispatch");
        let nats_run = mailbox_store
            .claim_dispatch(&dispatch_delivery.1)
            .await
            .expect("claim post-activation NATS dispatch")
            .expect("NATS re-wake claims candidate");
        assert_eq!(nats_run.instance_revision_id, update_candidate_revision);
        assert!(
            mailbox_store
                .claim_dispatch(&dispatch_delivery.1)
                .await
                .expect("duplicate post-activation NATS claim")
                .is_none()
        );
        sqlx::query("UPDATE runs SET state = 'cleaned_up', outcome = 'succeeded' WHERE id = $1")
            .bind(nats_run.run_id.as_uuid())
            .execute(&pool)
            .await
            .expect("finish post-activation NATS proof run");
        dispatch_delivery
            .0
            .double_ack()
            .await
            .expect("ack post-activation NATS dispatch");
    }
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
                brokered_rule_copies: Vec::new(),
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
                brokered_rule_copies: Vec::new(),
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
                brokered_rule_copies: Vec::new(),
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
