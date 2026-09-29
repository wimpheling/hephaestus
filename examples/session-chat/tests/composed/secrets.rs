use super::{MODEL_RULE, MODEL_SECRET_VALUE};
use forge_domain::{OrganizationId, ProjectId};
use identity_domain::AuthenticatedIdentity;
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport,
};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
pub(crate) async fn seed_model_import(
    pool: &PgPool,
    organization: OrganizationId,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
    import_alias: &str,
) -> Uuid {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id,user_id,role) \
         VALUES ($1,$2,'secret_manager') ON CONFLICT DO NOTHING",
    )
    .bind(project.as_uuid())
    .bind(identity.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("session secret manager role");
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("session secret key"),
        ),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    );
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            identity,
            CreateSecret {
                command_key: crate::secret_command_key("session-chat-create", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization),
                name: SecretName::parse(format!("session_chat_{secret_id}"))
                    .expect("session secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(MODEL_SECRET_VALUE).expect("session secret value"),
            },
        )
        .await
        .expect("create session model secret");
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            identity,
            GrantAndAcceptSecretImport {
                command_key: crate::secret_command_key("session-chat-grant", import_id.as_uuid()),
                grant_id: SecretGrantId::new(),
                secret_id,
                target: SecretTarget::Project(project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.model.example")],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse(import_alias).expect("session model alias"),
            },
        )
        .await
        .expect("grant session model secret");
    import_id.as_uuid()
}

pub(crate) async fn seed_model_secret(
    pool: &PgPool,
    organization: OrganizationId,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
    instance: Uuid,
    revision: Uuid,
    attachment: Uuid,
) -> Uuid {
    seed_model_secret_with_rule(
        pool,
        organization,
        project,
        identity,
        instance,
        revision,
        attachment,
        MODEL_RULE,
        "model",
    )
    .await
    .0
}

#[allow(clippy::too_many_arguments)] // Secret binding IDs and target rule are all independent acceptance inputs.
pub(crate) async fn seed_model_secret_with_rule(
    pool: &PgPool,
    organization: OrganizationId,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
    instance: Uuid,
    revision: Uuid,
    attachment: Uuid,
    model_rule: Uuid,
    import_alias: &str,
) -> (Uuid, Uuid) {
    let import_id = SecretImportId::from_uuid(
        seed_model_import(pool, organization, project, identity, import_alias).await,
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("session secret key"),
        ),
        Arc::new(authz_postgres::PostgresMelangeAuthorizer),
    );
    let binding_id = AgentSecretBindingId::new();
    let bound_revision = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            identity,
            BindSecret {
                command_key: crate::secret_command_key("session-chat-bind", binding_id.as_uuid()),
                binding_id,
                instance_id: release_domain::AgentInstanceId::from_uuid(instance),
                expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(revision),
                new_revision_id: bound_revision,
                import_id,
                slot: SecretSlotKey::parse("model").expect("session model slot"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment],
                destinations: vec![String::from("api.model.example")],
            },
        )
        .await
        .expect("bind session model secret");
    service
        .declare_brokered_https_rule(
            identity,
            DeclareBrokeredHttpsRule {
                command_key: crate::secret_command_key("session-chat-rule", model_rule),
                rule_id: model_rule,
                binding_id,
                destination: String::from("https://api.model.example"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare session model rule");
    (bound_revision.as_uuid(), binding_id.as_uuid())
}
