use crate::support::{LifecycleState, SENTINEL, key};
use forge_domain::RepositoryId;
use secret_application::{BindSecret, SecretServiceError};
use secret_domain::{AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretSlotKey};
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn bind_and_validate(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let fixture = &state.fixture;
    let service = std::sync::Arc::clone(&state.service);
    let target_manager = state.target_manager.clone();
    let ordinary_member = state.ordinary_member.clone();
    let import_id = state.import_id.expect("project import");
    let repository_import_id = state.repository_import_id.expect("repository import");
    let repository_grant_id = state.repository_grant_id.expect("repository grant");
    let (instance_id, initial_revision_id, attachment_id, _initial_run_id) =
        crate::support::seed_instance(&pool, fixture).await;
    let second_repository_id = RepositoryId::new();
    sqlx::query(
        "INSERT INTO repositories
          (id, project_id, name, default_branch, is_public)
          VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(second_repository_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(format!("second-{second_repository_id}"))
    .execute(&pool)
    .await
    .expect("seed second attached repository");
    let second_attachment_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_attachments
          (id, instance_id, project_id, repository_id, ref_selector,
           trigger_policy, enabled, created_by)
          VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(second_attachment_id)
    .bind(instance_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(second_repository_id.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .execute(&pool)
    .await
    .expect("seed second attachment");
    let bound_revision_id = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &target_manager,
            BindSecret {
                command_key: key("bind", bound_revision_id.as_uuid()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: initial_revision_id,
                new_revision_id: bound_revision_id,
                import_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![second_attachment_id, attachment_id],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("independently authorized target manager should bind opaque import");
    let active: (Uuid, bool) = sqlx::query_as(
        "SELECT instance.active_revision_id, revision.runnable
          FROM agent_instances AS instance
          JOIN agent_instance_revisions AS revision
            ON revision.id = instance.active_revision_id
          WHERE instance.id = $1",
    )
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active bound revision");
    assert_eq!(active, (bound_revision_id.as_uuid(), true));
    for (slot, phase, label) in [
        ("normal_only", ExecutionPhase::Update, "normal-slot-update"),
        ("update_only", ExecutionPhase::Normal, "update-slot-normal"),
    ] {
        let wrong_phase = service
            .bind_secret(
                &target_manager,
                BindSecret {
                    command_key: key(label, Uuid::new_v4()),
                    binding_id: AgentSecretBindingId::new(),
                    instance_id,
                    expected_revision_id: bound_revision_id,
                    new_revision_id: release_domain::AgentInstanceRevisionId::new(),
                    import_id,
                    slot: SecretSlotKey::parse(slot).expect("slot should validate"),
                    mode: DeliveryMode::Brokered,
                    phases: vec![phase],
                    attachment_ids: if phase == ExecutionPhase::Normal {
                        vec![attachment_id]
                    } else {
                        Vec::new()
                    },
                    destinations: vec![String::from("api.example.test")],
                },
            )
            .await;
        assert!(matches!(
            wrong_phase,
            Err(SecretServiceError::BindingPolicyMismatch)
        ));
    }
    let mixed_repository_scope = service
        .bind_secret(
            &ordinary_member,
            BindSecret {
                command_key: key("repository-mixed-scope", Uuid::new_v4()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: bound_revision_id,
                new_revision_id: release_domain::AgentInstanceRevisionId::new(),
                import_id: repository_import_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id, second_attachment_id],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await;
    assert!(matches!(
        mixed_repository_scope,
        Err(SecretServiceError::BindingOutOfScope)
    ));
    let repository_bound_revision_id = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &ordinary_member,
            BindSecret {
                command_key: key(
                    "repository-exact-scope",
                    repository_bound_revision_id.as_uuid(),
                ),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: bound_revision_id,
                new_revision_id: repository_bound_revision_id,
                import_id: repository_import_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("repository import should bind only to its exact attachment");
    sqlx::query(
        "UPDATE secret_grants
          SET phases = ARRAY['update']::text[]
          WHERE id = $1",
    )
    .bind(repository_grant_id.as_uuid())
    .execute(&pool)
    .await
    .expect("narrow carried grant phase");
    let narrowed_grant = service
        .bind_secret(
            &ordinary_member,
            BindSecret {
                command_key: key("narrowed-carried-grant", Uuid::new_v4()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: repository_bound_revision_id,
                new_revision_id: release_domain::AgentInstanceRevisionId::new(),
                import_id: repository_import_id,
                slot: SecretSlotKey::parse("optional").expect("slot should validate"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect_err("a narrowed grant must invalidate the carried binding");
    assert!(matches!(
        &narrowed_grant,
        SecretServiceError::BindingPolicyMismatch
    ));
    assert_eq!(
        narrowed_grant.to_string(),
        "secret binding policy exceeds its declaration or grant"
    );
    assert!(!narrowed_grant.to_string().contains(SENTINEL));
    sqlx::query(
        "UPDATE secret_grants
          SET phases = ARRAY['normal']::text[]
          WHERE id = $1",
    )
    .bind(repository_grant_id.as_uuid())
    .execute(&pool)
    .await
    .expect("restore carried grant phase");
    sqlx::query(
        "UPDATE secret_imports
          SET status = 'revoked', revoked_at = now()
          WHERE id = $1",
    )
    .bind(repository_import_id.as_uuid())
    .execute(&pool)
    .await
    .expect("revoke carried import");
    let revoked_import = service
        .bind_secret(
            &ordinary_member,
            BindSecret {
                command_key: key("revoked-carried-import", Uuid::new_v4()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: repository_bound_revision_id,
                new_revision_id: release_domain::AgentInstanceRevisionId::new(),
                import_id: repository_import_id,
                slot: SecretSlotKey::parse("optional").expect("slot should validate"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect_err("a revoked import must invalidate the carried binding");
    assert!(matches!(
        &revoked_import,
        SecretServiceError::AuthorizationDenied
    ));
    assert_eq!(
        revoked_import.to_string(),
        "secret command is not authorized"
    );
    assert!(!revoked_import.to_string().contains(SENTINEL));
    sqlx::query(
        "UPDATE secret_imports
          SET status = 'active', revoked_at = NULL
          WHERE id = $1",
    )
    .bind(repository_import_id.as_uuid())
    .execute(&pool)
    .await
    .expect("restore carried import");
    let raw_escalation = service
        .bind_secret(
            &target_manager,
            BindSecret {
                command_key: key("raw-escalation", Uuid::new_v4()),
                binding_id: AgentSecretBindingId::new(),
                instance_id,
                expected_revision_id: repository_bound_revision_id,
                new_revision_id: release_domain::AgentInstanceRevisionId::new(),
                import_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                mode: DeliveryMode::Raw,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment_id],
                destinations: Vec::new(),
            },
        )
        .await;
    assert!(matches!(
        raw_escalation,
        Err(SecretServiceError::BindingPolicyMismatch)
    ));

    state.instance_id = Some(instance_id);
    state.initial_revision_id = Some(initial_revision_id);
    state.attachment_id = Some(attachment_id);
    state.second_attachment_id = Some(second_attachment_id);
    state.bound_revision_id = Some(bound_revision_id);
    state.repository_bound_revision_id = Some(repository_bound_revision_id);
}
