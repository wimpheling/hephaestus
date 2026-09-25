use super::*;

/// Creates the complete durable secret authority that the daemon resolves at
/// mailbox dispatch. The only plaintext in this test is passed directly into
/// the encrypted secret-store boundary and is then asserted only at the local
/// TLS upstream.
#[allow(clippy::too_many_lines)]
pub async fn seed_brokered_https_fixture(
    pool: &sqlx::PgPool,
    actor: UserId,
    organization_id: OrganizationId,
    project_id: uuid::Uuid,
    instance: &SeededInstance,
) -> BrokeredFixture {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(project_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("authorize golden owner to manage project secret imports");

    let identity = AuthenticatedIdentity::new(
        actor,
        golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("golden secret key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            &identity,
            CreateSecret {
                command_key: secret_command_key("create", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization_id),
                name: SecretName::parse(format!("golden_broker_{secret_id}"))
                    .expect("fixture secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(BROKERED_E2E_SENTINEL).expect("fixture secret value"),
            },
        )
        .await
        .expect("create encrypted brokered fixture secret");
    let grant_id = SecretGrantId::new();
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            &identity,
            GrantAndAcceptSecretImport {
                command_key: secret_command_key("grant-accept", import_id.as_uuid()),
                grant_id,
                secret_id,
                target: SecretTarget::Project(ProjectId::from_uuid(project_id)),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse("golden_broker").expect("fixture import alias"),
            },
        )
        .await
        .expect("grant and import brokered fixture secret");
    let binding_id = AgentSecretBindingId::new();
    let bound_revision = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &identity,
            BindSecret {
                command_key: secret_command_key("bind", binding_id.as_uuid()),
                binding_id,
                instance_id: release_domain::AgentInstanceId::from_uuid(instance.instance),
                expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(
                    instance.revision,
                ),
                new_revision_id: bound_revision,
                import_id,
                slot: SecretSlotKey::parse("model").expect("fixture slot"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![instance.attachment],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("bind brokered fixture secret to immutable release revision");
    service
        .declare_brokered_https_rule(
            &identity,
            DeclareBrokeredHttpsRule {
                command_key: secret_command_key("declare-rule", BROKERED_E2E_RULE_ID),
                rule_id: BROKERED_E2E_RULE_ID,
                binding_id,
                destination: String::from("https://api.example.test"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare immutable brokered HTTPS fixture rule");
    let rule = BrokeredSecretRule {
        id: BrokeredSecretRuleId::from_uuid(BROKERED_E2E_RULE_ID),
        binding_id: binding_id.as_uuid(),
        instance_revision_id: bound_revision.as_uuid(),
        secret_version_id: version_id.as_uuid(),
        destination: Some(
            ExactHttpsOrigin::parse("https://api.example.test").expect("fixture destination"),
        ),
        location: HttpInjectionLocation::OutboundHeaderPrefix {
            header: HeaderName::parse("authorization").expect("fixture header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    };
    BrokeredFixture {
        upstream: BrokeredTlsUpstream::start(rule).await,
        import_id: import_id.as_uuid(),
        version_id: version_id.as_uuid(),
    }
}

pub struct BrokeredFixture {
    pub upstream: BrokeredTlsUpstream,
    pub import_id: uuid::Uuid,
    pub version_id: uuid::Uuid,
}

pub fn secret_command_key(operation: &str, id: uuid::Uuid) -> SecretCommandKey {
    SecretCommandKey::derive(operation, &[id.as_bytes()])
}

/// Certificate-valid loopback upstream available only to this daemon golden.
/// Its local address is accepted solely through `secret-broker`'s feature-gated
/// test fixture; the production registry still rejects private pins.
pub struct BrokeredTlsUpstream {
    pub adapter: Arc<dyn secret_application::BrokerAdapter>,
    pub cooking_registry: Option<Arc<cooking::CookingAdapters>>,
    pub rule_adapters:
        Arc<std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>>>,
    pub observed: Arc<AtomicBool>,
    pub server: tokio::task::JoinHandle<()>,
    pub update_barrier: Option<Arc<CookingUpdateBarrier>>,
    pub revocation_barrier: Option<Arc<CookingUpdateBarrier>>,
}
