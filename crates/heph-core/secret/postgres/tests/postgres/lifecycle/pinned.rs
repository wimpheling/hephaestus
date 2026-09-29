use crate::support::seed_queued_run;
use crate::support::{LifecycleState, ObservedBroker, key, role_pool};
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{CommitSha, GitRef};
use release_domain::AgentAttachmentId;
use secret_application::{BrokerRequest, ResolveRunSecrets, SecretServiceError};
use secret_domain::{ExecutionPhase, SecretRuntimeSessionId, SecretSlotKey};
use secret_postgres::SecretRuntimeService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn prepare_pinned_leases(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let service = std::sync::Arc::clone(&state.service);
    let target_manager = state.target_manager.clone();
    let instance_id = state.instance_id.expect("instance");
    let repository_bound_revision_id = state.repository_bound_revision_id.expect("bound revision");
    let attachment_id = state.attachment_id.expect("attachment");
    let brokered_rule_id = state.brokered_rule_id.expect("rule");
    let app_authorization_pool = role_pool("hephaestus_app").await;
    let worker_resolver_pool = role_pool("hephaestus_worker").await;
    let app_runtime = SecretRuntimeService::new(
        app_authorization_pool,
        worker_resolver_pool,
        EncryptedStore::new(
            LocalKeyProvider::new(
                "test/v1",
                [("test/v1", [7_u8; 32]), ("test/v2", [8_u8; 32])],
            )
            .expect("application runtime fixture keys should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );

    // The update fixture has closed the instance gate by this point. Restore
    // the pre-update state only long enough to issue these two independent
    // pinned leases; the update lifecycle is restored before the paused call
    // is released.
    sqlx::query(
        "UPDATE agent_instances
          SET active_revision_id = $2, state = 'active', run_gate_open = true
          WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .bind(repository_bound_revision_id.as_uuid())
    .execute(&pool)
    .await
    .expect("open gate for pinned lease regression");

    // Keep two old-version leases alive across rotation. The first adapter
    // call pauses after preauthorization; rotation must not invalidate its
    // pinned snapshot before the response is committed.
    let pinned_run = seed_queued_run(
        &pool,
        instance_id,
        repository_bound_revision_id,
        attachment_id,
    )
    .await;
    let pinned_authority = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("pinned-resolve", pinned_run.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: pinned_run,
                instance_id,
                instance_revision_id: repository_bound_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("d".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await
        .expect("old-version lease should resolve before rotation");
    let revoked_pinned_run = seed_queued_run(
        &pool,
        instance_id,
        repository_bound_revision_id,
        attachment_id,
    )
    .await;
    let revoked_pinned_authority = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("revoked-pinned-resolve", revoked_pinned_run.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: revoked_pinned_run,
                instance_id,
                instance_revision_id: repository_bound_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("e".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await
        .expect("second old-version lease should resolve before rotation");
    let pre_revoked_run = seed_queued_run(
        &pool,
        instance_id,
        repository_bound_revision_id,
        attachment_id,
    )
    .await;
    let pre_revoked_authority = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("pre-revoked-resolve", pre_revoked_run.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: pre_revoked_run,
                instance_id,
                instance_revision_id: repository_bound_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("f".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await
        .expect("pre-revoked HTTPS lease should resolve before revoke");
    let pre_revoked_lease_id = pre_revoked_authority.leases[0].lease_id.as_uuid();
    sqlx::query("UPDATE secret_leases SET status = 'revoked' WHERE id = $1")
        .bind(pre_revoked_lease_id)
        .execute(&pool)
        .await
        .expect("revoke exact pre-admission lease");
    let pre_adapter_observed = Arc::new(AtomicBool::new(false));
    let pre_adapter_result = app_runtime
        .use_brokered(
            &pre_revoked_authority.credential,
            &BrokerRequest {
                run_id: pre_revoked_run,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("https_v1"),
                body: serde_json::to_vec(&json!({ "rule_id": brokered_rule_id }))
                    .expect("pre-admission brokered HTTPS request"),
            },
            &ObservedBroker {
                observed: Arc::clone(&pre_adapter_observed),
            },
        )
        .await;
    assert!(matches!(
        pre_adapter_result,
        Err(SecretServiceError::Unavailable)
    ));
    assert!(
        !pre_adapter_observed.load(Ordering::SeqCst),
        "revoked pre-admission lease must not invoke the broker adapter"
    );
    let pre_revoked_decision: (Uuid, String, Option<String>) = sqlx::query_as(
        "SELECT lease_snapshot_id, decision, reason_code
           FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2
            AND event_kind = 'authorization_decision' AND decision = 'deny'",
    )
    .bind(pre_revoked_run.as_uuid())
    .bind(brokered_rule_id)
    .fetch_one(&pool)
    .await
    .expect("pre-admission revoked HTTPS deny audit");
    let pre_revoked_snapshot_id: Uuid =
        sqlx::query_scalar("SELECT id FROM brokered_secret_lease_snapshots WHERE lease_id = $1")
            .bind(pre_revoked_lease_id)
            .fetch_one(&pool)
            .await
            .expect("pre-admission revoked HTTPS lease snapshot");
    assert_eq!(
        pre_revoked_decision,
        (
            pre_revoked_snapshot_id,
            String::from("deny"),
            Some(String::from("live_authorization_unavailable"))
        )
    );
    let pre_revoked_substitution_uses: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2 AND event_kind = 'substitution_use'",
    )
    .bind(pre_revoked_run.as_uuid())
    .bind(brokered_rule_id)
    .fetch_one(&pool)
    .await
    .expect("pre-admission revoked HTTPS substitution audit");
    assert_eq!(pre_revoked_substitution_uses, 0);
    sqlx::query(
        "UPDATE secret_runtime_sessions
            SET status = 'revoked', revoked_at = now()
          WHERE id = $1",
    )
    .bind(pre_revoked_authority.session_id.as_uuid())
    .execute(&pool)
    .await
    .expect("revoke exact pre-admission runtime session");
    let revoked_session_result = app_runtime
        .use_brokered(
            &pre_revoked_authority.credential,
            &BrokerRequest {
                run_id: pre_revoked_run,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("https_v1"),
                body: serde_json::to_vec(&json!({ "rule_id": brokered_rule_id }))
                    .expect("revoked-session brokered HTTPS request"),
            },
            &ObservedBroker {
                observed: Arc::clone(&pre_adapter_observed),
            },
        )
        .await;
    assert!(matches!(
        revoked_session_result,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));
    assert!(
        !pre_adapter_observed.load(Ordering::SeqCst),
        "revoked runtime session must not invoke the broker adapter"
    );
    let revoked_session_denies: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2
            AND event_kind = 'authorization_decision' AND decision = 'deny'",
    )
    .bind(pre_revoked_run.as_uuid())
    .bind(brokered_rule_id)
    .fetch_one(&pool)
    .await
    .expect("revoked-session HTTPS deny audits");
    assert_eq!(revoked_session_denies, 2);
    state.pinned_run = Some(pinned_run);
    state.pinned_authority = Some(pinned_authority);
    state.revoked_pinned_run = Some(revoked_pinned_run);
    state.revoked_pinned_authority = Some(revoked_pinned_authority);
    state.pre_revoked_run = Some(pre_revoked_run);
    state.pre_revoked_authority = Some(pre_revoked_authority);
}
