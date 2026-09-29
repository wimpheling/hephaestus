use crate::support::{LifecycleState, key};
use secret_application::{
    AcceptSecretImport, CreateSecret, GrantAndAcceptSecretImport, GrantSecret, SecretServiceError,
};
use secret_domain::{
    DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId, SecretImportId, SecretName,
    SecretOwner, SecretTarget, SecretUsePolicy, SecretValue, SecretVersionId,
};

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn authorize_and_bind_setup(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let fixture = &state.fixture;
    let service = std::sync::Arc::clone(&state.service);
    let owner = state.owner.clone();
    let target_manager = state.target_manager.clone();
    let ordinary_member = state.ordinary_member.clone();
    let secret_id = state.secret_id;

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
    state.import_id = Some(import_id);
    state.repository_import_id = Some(repository_import_id);
    state.repository_grant_id = Some(repository_grant_id);
}
