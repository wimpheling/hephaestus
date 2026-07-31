//! Opt-in real-PostgreSQL secret lifecycle and non-disclosure coverage.

use authz_postgres::PostgresMelangeAuthorizer;
use forge_domain::{CommitSha, GitRef, ProjectId, RepositoryId};
use identity_domain::{AuthenticatedIdentity, OrganizationId, RequestId, UserId};
use release_domain::AgentAttachmentId;
use runtime_types::RunId;
use secret_application::{
    AcceptSecretImport, BindSecret, BrokerAdapter, BrokerAdapterError, BrokerRequest,
    BrokerResponse, BrokerStatus, CreateSecret, GrantAndAcceptSecretImport, GrantSecret,
    ResolveRunSecrets, RotateSecret, SecretServiceError,
};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretRuntimeSessionId,
    SecretSlotKey, SecretTarget, SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_postgres::{SecretRuntimeService, SecretService};
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

const SENTINEL: &str = "postgres-secret-sentinel-2747dcb8";

struct Fixture {
    owner: UserId,
    organization_secret_manager: UserId,
    ordinary_member: UserId,
    target_manager: UserId,
    other_owner: UserId,
    organization: OrganizationId,
    target_project: ProjectId,
    target_repository: RepositoryId,
    other_project: ProjectId,
}

struct FakeBroker {
    observed: AtomicBool,
    expected_credential: Vec<u8>,
}

#[async_trait::async_trait]
impl BrokerAdapter for FakeBroker {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        self.observed.store(
            credential.expose() == self.expected_credential
                && destination == "api.example.test"
                && operation == "complete"
                && body == b"bounded request",
            Ordering::SeqCst,
        );
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{"result":"sanitized"}"#.to_vec(),
        })
    }
}

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
// This end-to-end lifecycle intentionally keeps one fixture and transaction chain.
#[allow(clippy::large_stack_frames)]
async fn encrypted_rotation_delegation_revocation_and_purge_are_atomic() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let fixture = seed(&pool).await;
    let service = Arc::new(SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new(
                "test/v1",
                [("test/v1", [7_u8; 32]), ("test/v2", [8_u8; 32])],
            )
            .expect("fixture keys should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    ));
    let owner = identity(fixture.owner);
    let target_manager = identity(fixture.target_manager);
    let organization_secret_manager = identity(fixture.organization_secret_manager);
    let ordinary_member = identity(fixture.ordinary_member);
    let outsider = identity(fixture.other_owner);
    let manager_secret_id = SecretId::new();
    service
        .create(
            &organization_secret_manager,
            CreateSecret {
                command_key: key("manager-create", manager_secret_id.as_uuid()),
                secret_id: manager_secret_id,
                version_id: SecretVersionId::new(),
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("manager-{manager_secret_id}").replace('-', "_"))
                    .expect("manager secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new("manager-only-value").expect("manager secret value"),
            },
        )
        .await
        .expect("explicit organization secret manager should create");
    for (identity, label) in [
        (&ordinary_member, "ordinary-member"),
        (&outsider, "cross-tenant-outsider"),
    ] {
        let denied_secret_id = SecretId::new();
        let denied = service
            .create(
                identity,
                CreateSecret {
                    command_key: key(label, denied_secret_id.as_uuid()),
                    secret_id: denied_secret_id,
                    version_id: SecretVersionId::new(),
                    owner: SecretOwner::Organization(fixture.organization),
                    name: SecretName::parse(format!("denied-{denied_secret_id}").replace('-', "_"))
                        .expect("denied secret name"),
                    allowed_delivery_modes: vec![DeliveryMode::Brokered],
                    value: SecretValue::new("must-not-persist").expect("denied secret value"),
                },
            )
            .await;
        assert!(matches!(
            denied,
            Err(SecretServiceError::AuthorizationDenied)
        ));
    }
    let secret_id = SecretId::new();
    let first_version = SecretVersionId::new();
    let create_key = key("create", secret_id.as_uuid());
    let created = service
        .create(
            &owner,
            CreateSecret {
                command_key: create_key,
                secret_id,
                version_id: first_version,
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("model-{secret_id}").replace('-', "_"))
                    .expect("fixture name should validate"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered, DeliveryMode::Raw],
                value: SecretValue::new(SENTINEL).expect("sentinel should validate"),
            },
        )
        .await
        .expect("organization owner should create");
    assert_eq!(created.secret_id, secret_id);
    assert_eq!(created.version_id, first_version);

    let duplicate = service
        .create(
            &owner,
            CreateSecret {
                command_key: create_key,
                secret_id,
                version_id: first_version,
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("model-{secret_id}").replace('-', "_"))
                    .expect("fixture name should validate"),
                allowed_delivery_modes: vec![DeliveryMode::Raw, DeliveryMode::Brokered],
                value: SecretValue::new(SENTINEL).expect("sentinel should validate"),
            },
        )
        .await
        .expect("duplicate create should return durable result");
    assert_eq!(duplicate, created);

    let stored: (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT ciphertext, wrapped_data_key, associated_data_hash
          FROM secret_versions WHERE id = $1",
    )
    .bind(first_version.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored encrypted version");
    for bytes in [&stored.0, &stored.1, &stored.2] {
        assert!(
            !bytes
                .windows(SENTINEL.len())
                .any(|value| value == SENTINEL.as_bytes())
        );
    }
    let durable_text: String = sqlx::query_scalar(
        "SELECT concat_ws(' ',
             (SELECT string_agg(row_to_json(secrets)::text, ' ') FROM secrets
              WHERE id = $1),
             (SELECT string_agg(payload::text, ' ') FROM outbox
              WHERE aggregate_id = $1),
             (SELECT string_agg(row_to_json(secret_audit_events)::text, ' ')
              FROM secret_audit_events WHERE secret_id = $1))",
    )
    .bind(secret_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("inspect non-ciphertext durable records");
    assert!(!durable_text.contains(SENTINEL));

    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.owner.as_uuid())
    .execute(&pool)
    .await
    .expect("independently authorize owner on target side");
    let atomic_grant_id = SecretGrantId::new();
    let atomic_import_id = SecretImportId::new();
    let atomic_key = key("grant-accept", atomic_import_id.as_uuid());
    let atomic_command = GrantAndAcceptSecretImport {
        command_key: atomic_key,
        grant_id: atomic_grant_id,
        secret_id,
        target: SecretTarget::Project(fixture.target_project),
        policy: SecretUsePolicy {
            delivery_modes: vec![DeliveryMode::Brokered],
            phases: vec![ExecutionPhase::Normal],
            destinations: vec![String::from("api.example.test")],
        },
        expires_at: None,
        import_id: atomic_import_id,
        alias: SecretAlias::parse("atomic_model").expect("alias should validate"),
    };
    assert_eq!(
        service
            .grant_and_accept_import(&owner, atomic_command.clone())
            .await
            .expect("organization owner should pass both sides"),
        atomic_import_id
    );
    assert_eq!(
        service
            .grant_and_accept_import(&owner, atomic_command)
            .await
            .expect("compound retry should be idempotent"),
        atomic_import_id
    );
    let denied_grant_id = SecretGrantId::new();
    let denied_import_id = SecretImportId::new();
    let denied = service
        .grant_and_accept_import(
            &target_manager,
            GrantAndAcceptSecretImport {
                command_key: key("grant-accept-denied", denied_import_id.as_uuid()),
                grant_id: denied_grant_id,
                secret_id,
                target: SecretTarget::Project(fixture.target_project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
                import_id: denied_import_id,
                alias: SecretAlias::parse("denied_atomic").expect("alias should validate"),
            },
        )
        .await;
    assert!(matches!(
        denied,
        Err(SecretServiceError::AuthorizationDenied)
    ));
    let denied_rows: i64 = sqlx::query_scalar(
        "SELECT
              (SELECT count(*) FROM secret_grants WHERE id = $1)
            + (SELECT count(*) FROM secret_imports WHERE id = $2)",
    )
    .bind(denied_grant_id.as_uuid())
    .bind(denied_import_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("inspect atomic denial");
    assert_eq!(denied_rows, 0);

    let grant_id = SecretGrantId::new();
    service
        .grant(
            &owner,
            GrantSecret {
                command_key: key("grant", grant_id.as_uuid()),
                grant_id,
                secret_id,
                target: SecretTarget::Project(fixture.target_project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
            },
        )
        .await
        .expect("owner should grant exact project");
    let maintainer_import_id = SecretImportId::new();
    let maintainer_import = service
        .accept_import(
            &ordinary_member,
            AcceptSecretImport {
                command_key: key("maintainer-accept", maintainer_import_id.as_uuid()),
                import_id: maintainer_import_id,
                grant_id,
                target: SecretTarget::Project(fixture.target_project),
                alias: SecretAlias::parse("maintainer_denied").expect("alias should validate"),
            },
        )
        .await;
    assert!(matches!(
        maintainer_import,
        Err(SecretServiceError::AuthorizationDenied)
    ));
    let import_id = SecretImportId::new();
    service
        .accept_import(
            &target_manager,
            AcceptSecretImport {
                command_key: key("accept", import_id.as_uuid()),
                import_id,
                grant_id,
                target: SecretTarget::Project(fixture.target_project),
                alias: SecretAlias::parse("model").expect("alias should validate"),
            },
        )
        .await
        .expect("target secret manager should accept without source authority");
    let repository_grant_id = SecretGrantId::new();
    service
        .grant(
            &owner,
            GrantSecret {
                command_key: key("repository-grant", repository_grant_id.as_uuid()),
                grant_id: repository_grant_id,
                secret_id,
                target: SecretTarget::Repository(fixture.target_repository),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
            },
        )
        .await
        .expect("owner should grant exact repository");
    sqlx::query(
        "INSERT INTO repository_managers (repository_id, user_id)
          VALUES ($1, $2)",
    )
    .bind(fixture.target_repository.as_uuid())
    .bind(fixture.ordinary_member.as_uuid())
    .execute(&pool)
    .await
    .expect("seed ordinary repository manager");
    let repository_import_id = SecretImportId::new();
    let repository_manager_denied = service
        .accept_import(
            &ordinary_member,
            AcceptSecretImport {
                command_key: key("repository-manager-denied", repository_import_id.as_uuid()),
                import_id: repository_import_id,
                grant_id: repository_grant_id,
                target: SecretTarget::Repository(fixture.target_repository),
                alias: SecretAlias::parse("repository_model").expect("alias should validate"),
            },
        )
        .await;
    assert!(matches!(
        repository_manager_denied,
        Err(SecretServiceError::AuthorizationDenied)
    ));
    sqlx::query(
        "INSERT INTO repository_secret_roles (repository_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(fixture.target_repository.as_uuid())
    .bind(fixture.ordinary_member.as_uuid())
    .execute(&pool)
    .await
    .expect("grant explicit repository secret role");
    service
        .accept_import(
            &ordinary_member,
            AcceptSecretImport {
                command_key: key("repository-manager-allowed", repository_import_id.as_uuid()),
                import_id: repository_import_id,
                grant_id: repository_grant_id,
                target: SecretTarget::Repository(fixture.target_repository),
                alias: SecretAlias::parse("repository_model").expect("alias should validate"),
            },
        )
        .await
        .expect("explicit repository secret manager should accept");
    let project_secret_id = SecretId::new();
    service
        .create(
            &target_manager,
            CreateSecret {
                command_key: key("project-secret", project_secret_id.as_uuid()),
                secret_id: project_secret_id,
                version_id: SecretVersionId::new(),
                owner: SecretOwner::Project(fixture.target_project),
                name: SecretName::parse(format!("project-{project_secret_id}").replace('-', "_"))
                    .expect("project secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new("project-scoped-value").expect("project secret value"),
            },
        )
        .await
        .expect("project secret manager should create project secret");
    let project_repository_grant_id = SecretGrantId::new();
    service
        .grant(
            &target_manager,
            GrantSecret {
                command_key: key(
                    "project-repository-grant",
                    project_repository_grant_id.as_uuid(),
                ),
                grant_id: project_repository_grant_id,
                secret_id: project_secret_id,
                target: SecretTarget::Repository(fixture.target_repository),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
            },
        )
        .await
        .expect("project secret should grant into its repository");
    let project_repository_import_id = SecretImportId::new();
    service
        .accept_import(
            &ordinary_member,
            AcceptSecretImport {
                command_key: key(
                    "project-repository-import",
                    project_repository_import_id.as_uuid(),
                ),
                import_id: project_repository_import_id,
                grant_id: project_repository_grant_id,
                target: SecretTarget::Repository(fixture.target_repository),
                alias: SecretAlias::parse("project_repository_model")
                    .expect("alias should validate"),
            },
        )
        .await
        .expect("repository secret manager should accept project secret");
    let (instance_id, initial_revision_id, attachment_id, _initial_run_id) =
        seed_instance(&pool, &fixture).await;
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
    let run_id = seed_queued_run(
        &pool,
        instance_id,
        repository_bound_revision_id,
        attachment_id,
    )
    .await;
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
    let runtime = SecretRuntimeService::new(
        pool.clone(),
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new(
                "test/v1",
                [("test/v1", [7_u8; 32]), ("test/v2", [8_u8; 32])],
            )
            .expect("runtime fixture keys should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let adapter = FakeBroker {
        observed: AtomicBool::new(false),
        expected_credential: SENTINEL.as_bytes().to_vec(),
    };
    let broker_response = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("complete"),
                body: b"bounded request".to_vec(),
            },
            &adapter,
        )
        .await
        .expect("broker should exercise host-only credential");
    assert_eq!(broker_response.status, BrokerStatus::Succeeded);
    assert!(
        !broker_response
            .body
            .windows(SENTINEL.len())
            .any(|value| value == SENTINEL.as_bytes())
    );
    assert!(adapter.observed.load(Ordering::SeqCst));
    let alternate_destination = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("other.example.test"),
                operation: String::from("complete"),
                body: Vec::new(),
            },
            &adapter,
        )
        .await;
    assert!(matches!(
        alternate_destination,
        Err(SecretServiceError::BrokerRequestDenied)
    ));

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

    let second = SecretVersionId::new();
    let third = SecretVersionId::new();
    let rotate_second = service.rotate(
        &owner,
        RotateSecret {
            command_key: key("rotate-a", second.as_uuid()),
            secret_id,
            expected_active_version_id: first_version,
            new_version_id: second,
            value: SecretValue::new(format!("{SENTINEL}-v2")).expect("replacement should validate"),
        },
    );
    let rotate_third = service.rotate(
        &owner,
        RotateSecret {
            command_key: key("rotate-b", third.as_uuid()),
            secret_id,
            expected_active_version_id: first_version,
            new_version_id: third,
            value: SecretValue::new(format!("{SENTINEL}-v3")).expect("replacement should validate"),
        },
    );
    let (second_result, third_result) = tokio::join!(rotate_second, rotate_third);
    assert_ne!(second_result.is_ok(), third_result.is_ok());
    assert!(
        matches!(second_result, Err(SecretServiceError::StaleActiveVersion))
            || matches!(third_result, Err(SecretServiceError::StaleActiveVersion))
    );
    let active_version: Uuid =
        sqlx::query_scalar("SELECT active_version_id FROM secrets WHERE id = $1")
            .bind(secret_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("active rotated version");
    let expected_rotated_value = if active_version == second.as_uuid() {
        format!("{SENTINEL}-v2")
    } else {
        assert_eq!(active_version, third.as_uuid());
        format!("{SENTINEL}-v3")
    };
    let pinned_before_rotation = runtime
        .receive_raw(
            &raw_authority.credential,
            raw_run_id,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await
        .expect("existing lease remains pinned after rotation");
    assert_eq!(raw_authority.leases[0].version_id, first_version);
    assert_eq!(pinned_before_rotation.value.expose(), SENTINEL.as_bytes());

    sqlx::query(
        "UPDATE agent_updates
          SET state = 'rejected', final_decision = 'agent_rejected',
              completed_at = now(), updated_at = now()
          WHERE instance_id = $1 AND state = 'hook_running'",
    )
    .bind(instance_id.as_uuid())
    .execute(&pool)
    .await
    .expect("finish update fixture");
    sqlx::query(
        "UPDATE agent_instances
          SET active_revision_id = $2, state = 'active', run_gate_open = true
          WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .bind(raw_revision_id.as_uuid())
    .execute(&pool)
    .await
    .expect("restore normal dispatch fixture");
    let concurrent_run_a =
        seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let concurrent_run_b =
        seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let resolve_a = service.resolve_for_dispatch(
        &target_manager,
        ResolveRunSecrets {
            command_key: key("concurrent-resolve-a", concurrent_run_a.as_uuid()),
            session_id: SecretRuntimeSessionId::new(),
            run_id: concurrent_run_a,
            instance_id,
            instance_revision_id: raw_revision_id,
            attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
            target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
            target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
            phase: ExecutionPhase::Normal,
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
        },
    );
    let resolve_b = service.resolve_for_dispatch(
        &target_manager,
        ResolveRunSecrets {
            command_key: key("concurrent-resolve-b", concurrent_run_b.as_uuid()),
            session_id: SecretRuntimeSessionId::new(),
            run_id: concurrent_run_b,
            instance_id,
            instance_revision_id: raw_revision_id,
            attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
            target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
            target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
            phase: ExecutionPhase::Normal,
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
        },
    );
    let (authority_a, authority_b) = tokio::join!(resolve_a, resolve_b);
    let authority_a = authority_a.expect("first concurrent dispatch");
    let authority_b = authority_b.expect("second concurrent dispatch");
    assert_eq!(authority_a.leases[0].version_id.as_uuid(), active_version);
    assert_eq!(authority_b.leases[0].version_id.as_uuid(), active_version);
    let rotated_raw = runtime
        .receive_raw(
            &authority_a.credential,
            concurrent_run_a,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await
        .expect("later dispatch uses rotated version");
    assert_eq!(
        rotated_raw.value.expose(),
        expected_rotated_value.as_bytes()
    );
    sqlx::query(
        "UPDATE secret_runtime_sessions
          SET issued_at = now() - interval '2 minutes',
              expires_at = now() - interval '1 minute'
          WHERE id = $1",
    )
    .bind(authority_b.session_id.as_uuid())
    .execute(&pool)
    .await
    .expect("expire exact runtime session");
    let expired_session = runtime
        .receive_raw(
            &authority_b.credential,
            concurrent_run_b,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    assert!(matches!(
        expired_session,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));
    sqlx::query(
        "UPDATE secret_leases SET status = 'revoked'
          WHERE id = $1",
    )
    .bind(authority_a.leases[0].lease_id.as_uuid())
    .execute(&pool)
    .await
    .expect("revoke exact lease");
    let stale_lease = runtime
        .receive_raw(
            &authority_a.credential,
            concurrent_run_a,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    match stale_lease {
        Err(SecretServiceError::RuntimeAuthenticationDenied | SecretServiceError::Unavailable) => {}
        Err(error) => panic!("unexpected stale lease error: {error}"),
        Ok(_) => panic!("revoked lease unexpectedly remained usable"),
    }
    let version_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM secret_versions WHERE secret_id = $1")
            .bind(secret_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("version count");
    assert_eq!(version_count, 2);

    service
        .revoke_secret(&owner, key("revoke", secret_id.as_uuid()), secret_id)
        .await
        .expect("owner should revoke");
    let broker_after_revocation = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("complete"),
                body: Vec::new(),
            },
            &adapter,
        )
        .await;
    assert!(matches!(
        broker_after_revocation,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));
    let raw_after_revocation = runtime
        .receive_raw(
            &raw_authority.credential,
            raw_run_id,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await;
    assert!(raw_after_revocation.is_err());
    let post_revoke_run = seed_queued_run(&pool, instance_id, raw_revision_id, attachment_id).await;
    let before_resolution_denied = service
        .resolve_for_dispatch(
            &target_manager,
            ResolveRunSecrets {
                command_key: key("resolve-after-revoke", post_revoke_run.as_uuid()),
                session_id: SecretRuntimeSessionId::new(),
                run_id: post_revoke_run,
                instance_id,
                instance_revision_id: raw_revision_id,
                attachment_id: Some(AgentAttachmentId::from_uuid(attachment_id)),
                target_ref: Some(GitRef::parse("refs/heads/main").expect("target ref")),
                target_commit: Some(CommitSha::parse("b".repeat(40)).expect("target commit")),
                phase: ExecutionPhase::Normal,
                expires_at: time::OffsetDateTime::now_utc() + time::Duration::minutes(5),
            },
        )
        .await;
    assert!(before_resolution_denied.is_err());
    let revoked_runtime_state: (i64, i64, bool) = sqlx::query_as(
        "SELECT
              (SELECT count(*)::bigint FROM secret_runtime_sessions AS session
               JOIN secret_leases AS lease ON lease.session_id = session.id
               JOIN secret_versions AS version ON version.id = lease.secret_version_id
               WHERE version.secret_id = $1 AND session.status = 'active'),
              (SELECT count(*)::bigint FROM secret_leases AS lease
               JOIN secret_versions AS version ON version.id = lease.secret_version_id
               WHERE version.secret_id = $1 AND lease.status = 'active'),
              EXISTS(
                  SELECT 1 FROM secret_leases AS lease
                  JOIN secret_versions AS version ON version.id = lease.secret_version_id
                  WHERE version.secret_id = $1
                    AND lease.delivery_mode = 'raw'
                    AND lease.raw_material_observed
              )",
    )
    .bind(secret_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("revoked runtime authority");
    assert_eq!(revoked_runtime_state, (0, 0, true));
    let import_status: String =
        sqlx::query_scalar("SELECT status FROM secret_imports WHERE id = $1")
            .bind(import_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("import status");
    assert_eq!(import_status, "revoked");
    service
        .purge_secret(&owner, key("purge", secret_id.as_uuid()), secret_id)
        .await
        .expect("revoked secret without active leases should purge");
    let remaining_ciphertext: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM secret_versions
          WHERE secret_id = $1 AND (
             ciphertext IS NOT NULL OR wrapped_data_key IS NOT NULL
             OR data_nonce IS NOT NULL OR wrap_nonce IS NOT NULL
          )",
    )
    .bind(secret_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("purged encrypted material");
    assert_eq!(remaining_ciphertext, 0);
    let status: String = sqlx::query_scalar("SELECT status FROM secrets WHERE id = $1")
        .bind(secret_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("purged tombstone");
    assert_eq!(status, "purged");
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

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.secret.test",
        format!("secret-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

fn key(operation: &str, id: Uuid) -> SecretCommandKey {
    SecretCommandKey::derive(operation, &[id.as_bytes()])
}

async fn seed(pool: &PgPool) -> Fixture {
    let fixture = Fixture {
        owner: UserId::new(),
        organization_secret_manager: UserId::new(),
        ordinary_member: UserId::new(),
        target_manager: UserId::new(),
        other_owner: UserId::new(),
        organization: OrganizationId::new(),
        target_project: ProjectId::new(),
        target_repository: RepositoryId::new(),
        other_project: ProjectId::new(),
    };
    let other_organization = OrganizationId::new();
    seed_users(pool, &fixture).await;
    sqlx::query(
        "INSERT INTO organizations (id, name)
          VALUES ($1, $3), ($2, $4)",
    )
    .bind(fixture.organization.as_uuid())
    .bind(other_organization.as_uuid())
    .bind(format!("secret-org-{}", fixture.organization))
    .bind(format!("other-org-{other_organization}"))
    .execute(pool)
    .await
    .expect("seed organizations");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
          VALUES ($1, $2, 'owner'), ($1, $3, 'member'), ($1, $4, 'member'),
                 ($5, $6, 'owner')",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.owner.as_uuid())
    .bind(fixture.organization_secret_manager.as_uuid())
    .bind(fixture.ordinary_member.as_uuid())
    .bind(other_organization.as_uuid())
    .bind(fixture.other_owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed owners");
    sqlx::query(
        "INSERT INTO organization_secret_managers (organization_id, user_id)
          VALUES ($1, $2)",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.organization_secret_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed organization secret manager");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name)
          VALUES ($1, $2, $3), ($4, $5, $6)",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.organization.as_uuid())
    .bind(format!("target-{}", fixture.target_project))
    .bind(fixture.other_project.as_uuid())
    .bind(other_organization.as_uuid())
    .bind(format!("other-{}", fixture.other_project))
    .execute(pool)
    .await
    .expect("seed projects");
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed target manager");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
          VALUES ($1, $2), ($1, $3)",
    )
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .bind(fixture.ordinary_member.as_uuid())
    .execute(pool)
    .await
    .expect("seed instance manager");
    sqlx::query(
        "INSERT INTO repositories
          (id, project_id, name, default_branch, is_public)
          VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(fixture.target_repository.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(format!("target-{}", fixture.target_repository))
    .execute(pool)
    .await
    .expect("seed target repository");
    fixture
}

async fn seed_users(pool: &PgPool, fixture: &Fixture) {
    for (user, name) in [
        (fixture.owner, "secret-owner"),
        (
            fixture.organization_secret_manager,
            "organization-secret-manager",
        ),
        (fixture.ordinary_member, "ordinary-member"),
        (fixture.target_manager, "target-secret-manager"),
        (fixture.other_owner, "other-owner"),
    ] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(format!("{name}-{user}"))
            .execute(pool)
            .await
            .expect("seed user");
    }
}

async fn seed_instance(
    pool: &PgPool,
    fixture: &Fixture,
) -> (
    release_domain::AgentInstanceId,
    release_domain::AgentInstanceRevisionId,
    Uuid,
    RunId,
) {
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let instance_id = release_domain::AgentInstanceId::new();
    let revision_id = release_domain::AgentInstanceRevisionId::new();
    let attachment_id = Uuid::new_v4();
    seed_release_graph(
        pool,
        fixture,
        build_id,
        release_id,
        release_agent_id,
        family_id,
    )
    .await;
    seed_instance_revision(
        pool,
        fixture,
        instance_id,
        revision_id,
        release_agent_id,
        family_id,
    )
    .await;
    seed_attachment(pool, fixture, instance_id, attachment_id).await;
    let run_id = seed_queued_run(pool, instance_id, revision_id, attachment_id).await;
    (instance_id, revision_id, attachment_id, run_id)
}

async fn seed_queued_run(
    pool: &PgPool,
    instance_id: release_domain::AgentInstanceId,
    revision_id: release_domain::AgentInstanceRevisionId,
    attachment_id: Uuid,
) -> RunId {
    let run_id = RunId::new();
    let provenance: (Uuid, Uuid) = sqlx::query_as(
        "SELECT agent.release_id, agent.id
          FROM agent_instance_revisions AS revision
          JOIN release_agents AS agent ON agent.id = revision.release_agent_id
          WHERE revision.id = $1 AND revision.instance_id = $2",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("run release provenance");
    sqlx::query(
        "INSERT INTO runs
          (id, instance_id, instance_revision_id, release_id, release_agent_id,
           attachment_id, run_kind, command_id, state, requires_state,
           created_at, updated_at)
          VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'queued', true, now(), now())",
    )
    .bind(run_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(revision_id.as_uuid())
    .bind(provenance.0)
    .bind(provenance.1)
    .bind(attachment_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed queued run");
    run_id
}

async fn seed_release_graph(
    pool: &PgPool,
    fixture: &Fixture,
    build_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    family_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO build_requests
          (id, repository_id, source_commit, source_ref,
           build_definition_hash, state)
          VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(fixture.target_repository.as_uuid())
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
          VALUES ($1, $2, 'reviewer')",
    )
    .bind(family_id)
    .bind(fixture.target_repository.as_uuid())
    .execute(pool)
    .await
    .expect("seed agent family");
    sqlx::query(
        "INSERT INTO releases
          (id, repository_id, version, source_commit, source_ref,
           build_request_id, build_definition_hash, configuration,
           configuration_hash, manifest_hash, state, published_at)
          VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                  '{}', $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(fixture.target_repository.as_uuid())
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_agents
          (id, release_id, family_id, agent_key, display_name,
           runtime_contract, runtime_contract_hash, parameter_schema,
           secret_slot_schema, requires_state)
          VALUES ($1, $2, $3, 'reviewer', 'Reviewer', $4, $5, '[]', $6, false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(json!({
        "policy_ceiling": {
            "vcpus": 2,
            "memory_mib": 1024,
            "network": "broker_only"
        }
    }))
    .bind([4_u8; 32].as_slice())
    .bind(json!([
        {
            "key": "model",
            "purpose": "Call model",
            "required": true,
            "delivery_modes": ["brokered", "raw"],
            "phases": ["normal", "update"],
            "destinations": ["api.example.test"]
        },
        {
            "key": "optional",
            "purpose": "Optional integration",
            "required": false,
            "delivery_modes": ["brokered"],
            "phases": ["normal", "update"],
            "destinations": ["api.example.test"]
        },
        {
            "key": "normal_only",
            "purpose": "Normal-run integration",
            "required": false,
            "delivery_modes": ["brokered"],
            "phases": ["normal"],
            "destinations": ["api.example.test"]
        },
        {
            "key": "update_only",
            "purpose": "Update-hook integration",
            "required": false,
            "delivery_modes": ["brokered"],
            "phases": ["update"],
            "destinations": ["api.example.test"]
        }
    ]))
    .execute(pool)
    .await
    .expect("seed release agent");
}

async fn seed_instance_revision(
    pool: &PgPool,
    fixture: &Fixture,
    instance_id: release_domain::AgentInstanceId,
    revision_id: release_domain::AgentInstanceRevisionId,
    release_agent_id: Uuid,
    family_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_instances
          (id, project_id, family_id, name, state, active_revision_id,
           created_by)
          VALUES ($1, $2, $3, $4, 'active', NULL, $5)",
    )
    .bind(instance_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(family_id)
    .bind(format!("reviewer_{}", instance_id.as_uuid().simple()))
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
          (id, instance_id, release_agent_id, parameters, parameter_hash,
           secret_bindings, resource_selection, network_restriction,
           effective_runtime_policy, effective_policy_hash,
           platform_policy_version, runnable, diagnostics, created_by)
          VALUES ($1, $2, $3, '{}', $4, '[]', $5, $6, $5, $7,
                  'platform/v1', false, $8, $9)",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(json!({"vcpus": 1, "memory_mib": 512, "network": "broker_only"}))
    .bind(json!({"network": "broker_only"}))
    .bind([6_u8; 32].as_slice())
    .bind(json!([{
        "code": "required_secret_binding_missing",
        "field": "secret_slots.model"
    }]))
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed initial revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("activate initial revision");
}

async fn seed_attachment(
    pool: &PgPool,
    fixture: &Fixture,
    instance_id: release_domain::AgentInstanceId,
    attachment_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_attachments
          (id, instance_id, project_id, repository_id, ref_selector,
           trigger_policy, enabled, created_by)
          VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(attachment_id)
    .bind(instance_id.as_uuid())
    .bind(fixture.target_project.as_uuid())
    .bind(fixture.target_repository.as_uuid())
    .bind(fixture.target_manager.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
}
