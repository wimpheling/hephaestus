use crate::support::{LifecycleState, SENTINEL, key, seed_queued_run};
use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{CommitSha, GitRef};
use release_domain::AgentAttachmentId;
use runtime_types::RunId;
use secret_application::{
    AcceptSecretImport, BindSecret, GrantSecret, ResolveRunSecrets, SecretServiceError,
};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretImportId,
    SecretRuntimeSessionId, SecretSlotKey, SecretTarget, SecretUsePolicy,
};
use secret_postgres::SecretRuntimeService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use std::sync::Arc;
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines)]
pub(super) async fn prepare_raw_and_update(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let fixture = &state.fixture;
    let service = std::sync::Arc::clone(&state.service);
    let owner = state.owner.clone();
    let target_manager = state.target_manager.clone();
    let secret_id = state.secret_id;
    let instance_id = state.instance_id.expect("instance");
    let repository_bound_revision_id = state.repository_bound_revision_id.expect("bound revision");
    let attachment_id = state.attachment_id.expect("attachment");
    let run_id = state.run_id.expect("run");
    let runtime = SecretRuntimeService::new(
        pool.clone(),
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("test/v1", [("test/v1", [7_u8; 32])])
                .expect("runtime fixture keys"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );

    let raw_grant_id = SecretGrantId::new();
    service
        .grant(
            &owner,
            GrantSecret {
                command_key: key("raw-grant", raw_grant_id.as_uuid()),
                grant_id: raw_grant_id,
                secret_id,
                target: SecretTarget::Project(fixture.target_project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Raw],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: Vec::new(),
                },
                expires_at: None,
            },
        )
        .await
        .expect("owner should grant raw authority independently");
    let raw_import_id = SecretImportId::new();
    service
        .accept_import(
            &target_manager,
            AcceptSecretImport {
                command_key: key("raw-accept", raw_import_id.as_uuid()),
                import_id: raw_import_id,
                grant_id: raw_grant_id,
                target: SecretTarget::Project(fixture.target_project),
                alias: SecretAlias::parse("raw_model").expect("alias should validate"),
            },
        )
        .await
        .expect("target should accept raw import");
    let raw_revision_id = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &target_manager,
            BindSecret {
                command_key: key("raw-bind", raw_revision_id.as_uuid()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: repository_bound_revision_id,
                new_revision_id: raw_revision_id,
                import_id: raw_import_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                mode: DeliveryMode::Raw,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id],
                destinations: Vec::new(),
            },
        )
        .await
        .expect("independent raw grant should create a new revision");
    let raw_run_id = seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let raw_authority = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("raw-resolve", raw_run_id.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: raw_run_id,
                instance_id,
                instance_revision_id: raw_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("c".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await
        .expect("raw dispatch should pin exact authority");
    let raw = runtime
        .receive_raw(
            &raw_authority.credential,
            raw_run_id,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await
        .expect("raw resolver should return only the exact ephemeral value");
    assert_eq!(raw.value.expose(), SENTINEL.as_bytes());
    let raw_observed: bool =
        sqlx::query_scalar("SELECT raw_material_observed FROM secret_leases WHERE id = $1")
            .bind(raw.lease_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("raw observation marker");
    assert!(raw_observed);
    let stolen_across_runs = runtime
        .receive_raw(
            &raw_authority.credential,
            run_id,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    assert!(matches!(
        stolen_across_runs,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));

    let update_grant_id = SecretGrantId::new();
    service
        .grant(
            &owner,
            GrantSecret {
                command_key: key("update-grant", update_grant_id.as_uuid()),
                grant_id: update_grant_id,
                secret_id,
                target: SecretTarget::Project(fixture.target_project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Update],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
            },
        )
        .await
        .expect("owner should grant update-only project authority");
    let update_import_id = SecretImportId::new();
    service
        .accept_import(
            &target_manager,
            AcceptSecretImport {
                command_key: key("update-import", update_import_id.as_uuid()),
                import_id: update_import_id,
                grant_id: update_grant_id,
                target: SecretTarget::Project(fixture.target_project),
                alias: SecretAlias::parse("model_update").expect("alias should validate"),
            },
        )
        .await
        .expect("target should accept update-only import");
    let update_revision_id = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &target_manager,
            BindSecret {
                command_key: key("update-bind", update_revision_id.as_uuid()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: raw_revision_id,
                new_revision_id: update_revision_id,
                import_id: update_import_id,
                slot: SecretSlotKey::parse("update_only").expect("slot should validate"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Update],
                attachment_ids: Vec::new(),
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("project import should bind to the instance-wide update phase");
    let update_run_id = RunId::new();
    let release_provenance: (Uuid, Uuid) = sqlx::query_as(
        "SELECT agent.release_id, agent.id
          FROM agent_instance_revisions AS revision
          JOIN release_agents AS agent ON agent.id = revision.release_agent_id
          WHERE revision.id = $1 AND revision.instance_id = $2",
    )
    .bind(update_revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("update release provenance");
    sqlx::query(
        "INSERT INTO runs
          (id, instance_id, instance_revision_id, release_id,
           release_agent_id, run_kind, command_id, state, requires_state,
           created_at, updated_at)
          VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', false,
                  now(), now())",
    )
    .bind(update_run_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(update_revision_id.as_uuid())
    .bind(release_provenance.0)
    .bind(release_provenance.1)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("seed exact update run");
    sqlx::query(
        "INSERT INTO agent_updates
          (id, instance_id, expected_current_revision_id,
           candidate_revision_id, state, hook_run_id, actor_id)
          VALUES ($1, $2, $3, $4, 'hook_running', $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id.as_uuid())
    .bind(raw_revision_id.as_uuid())
    .bind(update_revision_id.as_uuid())
    .bind(update_run_id.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .execute(&pool)
    .await
    .expect("seed exact update lifecycle");
    sqlx::query(
        "UPDATE agent_instances
          SET active_revision_id = $2, state = 'updating',
              run_gate_open = false
          WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .bind(raw_revision_id.as_uuid())
    .execute(&pool)
    .await
    .expect("close instance gate for update");
    let update_authority = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("update-resolve", update_run_id.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: update_run_id,
                instance_id,
                instance_revision_id: update_revision_id,
                attachment_id: None,
                target_ref: None,
                target_commit: None,
                phase: ExecutionPhase::Update,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await
        .expect("update dispatch should pin project-scoped secret authority");
    assert_eq!(update_authority.leases.len(), 1);

    let cross_tenant = service
        .grant(
            &owner,
            GrantSecret {
                command_key: key("cross", Uuid::new_v4()),
                grant_id: SecretGrantId::new(),
                secret_id,
                target: SecretTarget::Project(fixture.other_project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: Vec::new(),
                },
                expires_at: None,
            },
        )
        .await;
    assert!(matches!(
        cross_tenant,
        Err(SecretServiceError::CrossOrganization)
    ));

    state.raw_revision_id = Some(raw_revision_id);
    state.raw_run_id = Some(raw_run_id);
    state.raw_authority = Some(raw_authority);
    state.update_revision_id = Some(update_revision_id);
    state.update_run_id = Some(update_run_id);
    state.update_authority = Some(update_authority);
}
