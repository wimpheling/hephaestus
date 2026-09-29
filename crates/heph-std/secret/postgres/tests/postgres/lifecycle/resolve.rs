use crate::support::seed_queued_run;
use crate::support::{BrokeredRuleReceipt, LifecycleState, key};
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use forge_domain::{CommitSha, GitRef};
use identity_domain::RequestId;
use release_domain::AgentAttachmentId;
use secret_application::{DeclareBrokeredHttpsRule, ResolveRunSecrets, SecretServiceError};
use secret_domain::{AgentSecretBindingId, ExecutionPhase, SecretRuntimeSessionId};
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn resolve_and_publish(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let fixture = &state.fixture;
    let service = std::sync::Arc::clone(&state.service);
    let target_manager = state.target_manager.clone();
    let ordinary_member = state.ordinary_member.clone();
    let first_version = state.first_version;
    let instance_id = state.instance_id.expect("instance");
    let repository_bound_revision_id = state.repository_bound_revision_id.expect("bound revision");
    let attachment_id = state.attachment_id.expect("attachment");
    let run_id = seed_queued_run(
        &pool,
        instance_id,
        repository_bound_revision_id,
        attachment_id,
    )
    .await;
    // Model the orchestrator's independent provenance ensure: secret dispatch
    // must consume and validate this row rather than own its target capture.
    sqlx::query(
        "INSERT INTO run_instance_provenance
           (run_id, instance_id, instance_revision_id, release_id,
            release_agent_id, attachment_id, target_repository_id,
            target_ref, target_commit, parameter_hash,
            platform_policy_version, phase, authorization_model_version)
         SELECT run.id, run.instance_id, run.instance_revision_id,
                run.release_id, run.release_agent_id, run.attachment_id,
                attachment.repository_id, 'refs/heads/main', repeat('b', 40),
                revision.parameter_hash, revision.platform_policy_version,
                'normal', $2
           FROM runs AS run
           JOIN agent_instance_revisions AS revision
             ON revision.id = run.instance_revision_id
            AND revision.instance_id = run.instance_id
           JOIN agent_attachments AS attachment
             ON attachment.id = run.attachment_id
            AND attachment.instance_id = run.instance_id
          WHERE run.id = $1",
    )
    .bind(run_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&pool)
    .await
    .expect("preexisting immutable dispatch provenance");
    let brokered_binding_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM agent_secret_bindings
          WHERE instance_revision_id = $1 AND slot_key = 'model' AND status = 'active'",
    )
    .bind(repository_bound_revision_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact active brokered binding");
    let previous_binding_event_version: i64 = sqlx::query_scalar(
        "SELECT COALESCE(max(aggregate_version), 0)
           FROM application_events
          WHERE aggregate_type = 'agent_secret_binding'
            AND aggregate_id = $1
            AND scope_kind = 'agent_instance'
            AND scope_id = $2",
    )
    .bind(brokered_binding_id)
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read binding event version before rule declaration");
    let previous_binding_occurrence_id: Uuid = sqlx::query_scalar(
        "SELECT occurrence_id
           FROM application_events
          WHERE aggregate_type = 'agent_secret_binding'
            AND aggregate_id = $1
            AND scope_kind = 'agent_instance'
            AND scope_id = $2
          ORDER BY cursor DESC
          LIMIT 1",
    )
    .bind(brokered_binding_id)
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("read binding occurrence before rule declaration");
    let rule_identity = ordinary_member
        .clone()
        .with_idempotency_id(RequestId::new());
    assert_ne!(
        rule_identity.idempotency_id.as_uuid(),
        previous_binding_occurrence_id
    );
    let brokered_rule_id = Uuid::new_v4();
    service
        .declare_brokered_https_rule(
            &rule_identity,
            DeclareBrokeredHttpsRule {
                command_key: key("declare-brokered-rule", brokered_rule_id),
                rule_id: brokered_rule_id,
                binding_id: AgentSecretBindingId::from_uuid(brokered_binding_id),
                destination: String::from("https://api.example.test"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare exact immutable brokered HTTPS rule");
    let receipt: BrokeredRuleReceipt = sqlx::query_as(
        "SELECT id, occurrence_id, actor_id, scope_id, actor_type, safe_state,
                    aggregate_type, scope_kind, event_type, change_kind,
                    aggregate_version, related_id_one, related_id_two
               FROM application_events
              WHERE occurrence_id = $1
                AND actor_id = $2
                AND aggregate_type = 'agent_secret_binding'
                AND scope_kind = 'agent_instance'
                AND scope_id = $4
                AND aggregate_id = $3
                AND actor_type = 'user'
                AND safe_state = 'active'
              ORDER BY cursor DESC
              LIMIT 1",
    )
    .bind(rule_identity.idempotency_id.as_uuid())
    .bind(rule_identity.user_id.as_uuid())
    .bind(brokered_binding_id)
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load brokered rule mutation receipt event");
    assert_eq!(receipt.1, rule_identity.idempotency_id.as_uuid());
    assert_eq!(receipt.2, rule_identity.user_id.as_uuid());
    assert_eq!(receipt.3, instance_id.as_uuid());
    assert_eq!(receipt.4, "user");
    assert_eq!(receipt.5.as_deref(), Some("active"));
    assert_eq!(receipt.6, "agent_secret_binding");
    assert_eq!(receipt.7, "agent_instance");
    assert_eq!(receipt.8, "agent_secret_binding.changed");
    assert_eq!(receipt.9, "updated");
    assert_eq!(receipt.10, previous_binding_event_version + 1);
    assert_eq!(receipt.11, instance_id.as_uuid());
    let binding_import_id: Uuid =
        sqlx::query_scalar("SELECT import_id FROM agent_secret_bindings WHERE id = $1")
            .bind(brokered_binding_id)
            .fetch_one(&pool)
            .await
            .expect("load brokered binding import relation");
    assert_eq!(receipt.12, binding_import_id);
    let outbox_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM product_event_outbox WHERE event_id = $1")
            .bind(receipt.0)
            .fetch_one(&pool)
            .await
            .expect("load committed brokered rule event outbox row");
    assert_eq!(outbox_count, 1);
    let replayed_rule = service
        .declare_brokered_https_rule(
            &rule_identity,
            DeclareBrokeredHttpsRule {
                command_key: key("declare-brokered-rule", brokered_rule_id),
                rule_id: Uuid::new_v4(),
                binding_id: AgentSecretBindingId::from_uuid(brokered_binding_id),
                destination: String::from("https://api.example.test"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("same command key should replay stored rule id");
    assert_eq!(replayed_rule, brokered_rule_id);
    let replay_event: (Uuid, i64) = sqlx::query_as(
        "SELECT id, count(*) OVER ()
           FROM application_events
          WHERE occurrence_id = $1
            AND actor_id = $2
            AND aggregate_type = 'agent_secret_binding'
            AND scope_kind = 'agent_instance'
            AND scope_id = $4
            AND aggregate_id = $3
            AND change_kind = 'updated'
            AND actor_type = 'user'
            AND safe_state = 'active'
          ORDER BY cursor DESC
          LIMIT 1",
    )
    .bind(rule_identity.idempotency_id.as_uuid())
    .bind(rule_identity.user_id.as_uuid())
    .bind(brokered_binding_id)
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("load replayed brokered rule receipt event");
    assert_eq!(replay_event.0, receipt.0);
    assert_eq!(replay_event.1, 1);
    let collision = service
        .declare_brokered_https_rule(
            &ordinary_member,
            DeclareBrokeredHttpsRule {
                command_key: key("declare-brokered-rule-collision", brokered_rule_id),
                rule_id: brokered_rule_id,
                binding_id: AgentSecretBindingId::from_uuid(brokered_binding_id),
                destination: String::from("https://api.example.test"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await;
    assert!(matches!(collision, Err(SecretServiceError::Persistence)));
    let resolve_key = key("resolve", run_id.as_uuid());
    let authority = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: resolve_key,
                session_id: SecretRuntimeSessionId::new(),
                run_id,
                instance_id,
                instance_revision_id: repository_bound_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await
        .expect("live exact authority should resolve at dispatch");
    assert_eq!(authority.leases.len(), 1);
    assert_eq!(authority.leases[0].version_id, first_version);
    crate::support::assert_https_inspection(&pool, fixture, run_id, first_version).await;
    let snapshot: (Uuid, String, String, String, Option<String>) = sqlx::query_as(
        "SELECT rule_id, destination_origin, location_kind, header_name, header_prefix
           FROM brokered_secret_lease_snapshots
          WHERE lease_id = $1",
    )
    .bind(authority.leases[0].lease_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact brokered rule snapshot");
    assert_eq!(
        snapshot,
        (
            brokered_rule_id,
            String::from("https://api.example.test"),
            String::from("outbound_header_prefix"),
            String::from("authorization"),
            Some(String::from("Bearer ")),
        )
    );
    assert_eq!(format!("{}", authority.credential), "[REDACTED]");
    let token_hash = authority.credential.storage_hash();
    let stored_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT runtime_credential_hash FROM secret_runtime_sessions
          WHERE run_id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored runtime token hash");
    assert_eq!(stored_hash, token_hash);
    assert_ne!(stored_hash, authority.credential.expose());
    let duplicate_resolution = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: resolve_key,
                session_id: SecretRuntimeSessionId::new(),
                run_id,
                instance_id,
                instance_revision_id: repository_bound_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await;
    assert!(matches!(
        duplicate_resolution,
        Err(SecretServiceError::CredentialAlreadyIssued)
    ));
    state.run_id = Some(run_id);
    state.brokered_binding_id = Some(brokered_binding_id);
    state.brokered_rule_id = Some(brokered_rule_id);
    state.authority = Some(authority);
}
