// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
#[allow(clippy::too_many_lines)]
pub(crate) async fn seed_brokered_fixture(
    pool: &sqlx::PgPool,
    actor: UserId,
    organization: OrganizationId,
    project: uuid::Uuid,
    instance: &SeededInstance,
) -> BrokeredFixture {
    sqlx::query("INSERT INTO project_secret_roles (project_id,user_id,role) VALUES ($1,$2,'secret_manager')")
        .bind(project).bind(actor.as_uuid()).execute(pool).await.expect("secret manager");
    let identity = AuthenticatedIdentity::new(
        actor,
        crate::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])]).expect("key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let mut revision = instance.revision;
    let mut rules = Vec::new();
    for (slot, host, sentinel) in [
        ("model", "api.model.example", MODEL_SENTINEL),
        ("telegram_relay", "relay.cooking.example", RELAY_SENTINEL),
    ] {
        let secret_id = SecretId::new();
        let version_id = SecretVersionId::new();
        service
            .create(
                &identity,
                CreateSecret {
                    command_key: secret_command_key("cooking-create", secret_id.as_uuid()),
                    secret_id,
                    version_id,
                    owner: SecretOwner::Organization(organization),
                    name: SecretName::parse(format!("cooking_{secret_id}")).expect("name"),
                    allowed_delivery_modes: vec![DeliveryMode::Brokered],
                    value: SecretValue::new(sentinel).expect("sentinel"),
                },
            )
            .await
            .expect("encrypted fixture credential");
        let import_id = SecretImportId::new();
        service
            .grant_and_accept_import(
                &identity,
                GrantAndAcceptSecretImport {
                    command_key: secret_command_key("cooking-grant", import_id.as_uuid()),
                    grant_id: SecretGrantId::new(),
                    secret_id,
                    target: SecretTarget::Project(ProjectId::from_uuid(project)),
                    policy: SecretUsePolicy {
                        delivery_modes: vec![DeliveryMode::Brokered],
                        phases: vec![ExecutionPhase::Normal],
                        destinations: vec![host.to_owned()],
                    },
                    expires_at: None,
                    import_id,
                    alias: SecretAlias::parse(slot).expect("alias"),
                },
            )
            .await
            .expect("grant and import");
        let binding_id = AgentSecretBindingId::new();
        let next = release_domain::AgentInstanceRevisionId::new();
        service
            .bind_secret(
                &identity,
                BindSecret {
                    command_key: secret_command_key("cooking-bind", binding_id.as_uuid()),
                    binding_id,
                    instance_id: release_domain::AgentInstanceId::from_uuid(instance.instance),
                    expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(
                        revision,
                    ),
                    new_revision_id: next,
                    import_id,
                    slot: SecretSlotKey::parse(slot).expect("slot"),
                    mode: DeliveryMode::Brokered,
                    phases: vec![ExecutionPhase::Normal],
                    attachment_ids: vec![instance.attachment],
                    destinations: vec![host.to_owned()],
                },
            )
            .await
            .expect("bind cooking slot");
        revision = next.as_uuid();
        let rule_id = if slot == "model" {
            MODEL_RULE
        } else {
            RELAY_RULE
        };
        rules.push(BrokeredSecretRule {
            id: BrokeredSecretRuleId::from_uuid(rule_id),
            binding_id: binding_id.as_uuid(),
            instance_revision_id: revision,
            secret_version_id: version_id.as_uuid(),
            destination: Some(ExactHttpsOrigin::parse(format!("https://{host}")).expect("origin")),
            location: HttpInjectionLocation::OutboundHeaderPrefix {
                header: HeaderName::parse("authorization").expect("header"),
                prefix: String::from("Bearer "),
            },
            gateway_route_id: None,
        });
    }
    for rule in &mut rules {
        let slot = if rule.id.as_uuid() == MODEL_RULE {
            "model"
        } else {
            "telegram_relay"
        };
        rule.binding_id = sqlx::query_scalar(
            "SELECT id FROM agent_secret_bindings WHERE instance_revision_id=$1 AND slot_key=$2",
        )
        .bind(revision)
        .bind(slot)
        .fetch_one(pool)
        .await
        .expect("final immutable binding");
        rule.instance_revision_id = revision;
        let host = if slot == "model" {
            "api.model.example"
        } else {
            "relay.cooking.example"
        };
        service
            .declare_brokered_https_rule(
                &identity,
                DeclareBrokeredHttpsRule {
                    command_key: secret_command_key("cooking-rule", rule.id.as_uuid()),
                    rule_id: rule.id.as_uuid(),
                    binding_id: AgentSecretBindingId::from_uuid(rule.binding_id),
                    destination: format!("https://{host}"),
                    header: String::from("authorization"),
                    header_prefix: Some(String::from("Bearer ")),
                },
            )
            .await
            .expect("declare final cooking rule");
    }
    let upstream = cooking_upstreams(rules).await;
    let (import_id, version_id) =
        seed_inbound_secret(&service, &identity, organization, project).await;
    BrokeredFixture {
        upstream,
        import_id,
        version_id,
    }
}

pub(crate) async fn seed_inbound_secret(
    service: &SecretService<LocalKeyProvider>,
    identity: &AuthenticatedIdentity,
    organization: OrganizationId,
    project: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            identity,
            CreateSecret {
                command_key: secret_command_key("cooking-inbound", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization),
                name: SecretName::parse(format!("cooking_inbound_{secret_id}")).expect("name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(INBOUND_SENTINEL).expect("inbound sentinel"),
            },
        )
        .await
        .expect("separate inbound secret");
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            identity,
            GrantAndAcceptSecretImport {
                command_key: secret_command_key("cooking-inbound-import", import_id.as_uuid()),
                grant_id: SecretGrantId::new(),
                secret_id,
                target: SecretTarget::Project(ProjectId::from_uuid(project)),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse("telegram_inbound").expect("inbound alias"),
            },
        )
        .await
        .expect("inbound-only import");
    (import_id.as_uuid(), version_id.as_uuid())
}

pub const INBOUND_SENTINEL: &str = "cooking-inbound-only-fixture-sentinel";
pub const MODEL_SENTINEL: &str = "cooking-model-only-fixture-sentinel-724c";
pub const RELAY_SENTINEL: &str = "cooking-relay-only-fixture-sentinel-819e";
