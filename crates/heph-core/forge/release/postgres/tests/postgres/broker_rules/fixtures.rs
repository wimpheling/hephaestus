use super::*;

pub(super) async fn seed_secret_manager_roles(pool: &PgPool, fixture: &Fixture) {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
         VALUES ($1, $2, 'secret_manager')",
    )
    .bind(fixture.first_project.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed secret manager role");
    sqlx::query(
        "INSERT INTO organization_secret_managers (organization_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed organization secret manager role");
}

pub(super) async fn publish_broker_instance(
    service: &ReleaseService,
    fixture: &Fixture,
    actor: &AuthenticatedIdentity,
) -> (
    ReleaseId,
    ReleaseAgentId,
    AgentInstanceId,
    AgentInstanceRevisionId,
    Uuid,
    Uuid,
    AgentAttachmentId,
) {
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    service
        .complete_build(CompleteBuild {
            command_key: key("broker-copy-complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("broker-copy-v1").expect("release version"),
            release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("bin/reviewer").expect("artifact path"),
                kind: ArtifactKind::Executable,
                mode: 0o555,
                content_hash: ContentHash::digest(b"broker-copy-artifact"),
                size_bytes: 20,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("complete broker-copy release");
    service
        .publish(
            actor,
            key("broker-copy-publish", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish broker-copy release");
    let instance_id = AgentInstanceId::new();
    let initial_revision = AgentInstanceRevisionId::new();
    let source_model_rule_id = Uuid::new_v4();
    let source_relay_rule_id = Uuid::new_v4();
    service
        .import_agent(
            actor,
            ImportAgent {
                command_key: key("broker-copy-import", instance_id.as_uuid()),
                instance_id,
                revision_id: initial_revision,
                project_id: fixture.first_project,
                release_agent_id,
                name: InstanceName::parse("broker-copy").expect("instance name"),
                parameters: broker_copy_parameters(
                    "warning",
                    source_model_rule_id,
                    source_relay_rule_id,
                ),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("import broker-copy instance");
    let attachment_id = AgentAttachmentId::new();
    service
        .create_attachment(
            actor,
            CreateAttachment {
                command_key: key("broker-copy-attachment", attachment_id.as_uuid()),
                attachment_id,
                instance_id,
                repository_id: fixture.first_repository,
                ref_selector: RefSelector::parse("refs/heads/main").expect("attachment ref"),
                trigger_policy: TriggerPolicy::Manual,
            },
        )
        .await
        .expect("create broker-copy attachment");

    (
        release_id,
        release_agent_id,
        instance_id,
        initial_revision,
        source_model_rule_id,
        source_relay_rule_id,
        attachment_id,
    )
}

pub(super) fn new_secret_service(pool: &PgPool) -> SecretService<LocalKeyProvider> {
    SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("broker-copy/v1", [("broker-copy/v1", [31_u8; 32])])
                .expect("secret key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    )
}

pub(super) async fn create_broker_secrets(
    secret_service: &SecretService<LocalKeyProvider>,
    fixture: &Fixture,
    actor: &AuthenticatedIdentity,
    instance_id: AgentInstanceId,
    initial_revision: AgentInstanceRevisionId,
    attachment_id: AgentAttachmentId,
) -> (
    AgentInstanceRevisionId,
    BTreeMap<String, (SecretId, SecretImportId, SecretVersionId, String)>,
) {
    let mut revision = initial_revision;
    let mut imports = BTreeMap::new();
    for (slot, host) in [("model", "api.example.test"), ("relay", "relay.example")] {
        let secret_id = SecretId::new();
        let version_id = SecretVersionId::new();
        secret_service
            .create(
                actor,
                CreateSecret {
                    command_key: secret_key("create", secret_id.as_uuid()),
                    secret_id,
                    version_id,
                    owner: SecretOwner::Organization(fixture.organization),
                    name: SecretName::parse(format!("broker_copy_{slot}")).expect("secret name"),
                    allowed_delivery_modes: vec![DeliveryMode::Brokered],
                    value: SecretValue::new(format!("broker-copy-{slot}-value"))
                        .expect("secret value"),
                },
            )
            .await
            .expect("create broker-copy secret");
        let import_id = SecretImportId::new();
        secret_service
            .grant_and_accept_import(
                actor,
                GrantAndAcceptSecretImport {
                    command_key: secret_key("grant", import_id.as_uuid()),
                    grant_id: SecretGrantId::new(),
                    secret_id,
                    target: SecretTarget::Project(fixture.first_project),
                    policy: SecretUsePolicy {
                        delivery_modes: vec![DeliveryMode::Brokered],
                        phases: vec![ExecutionPhase::Normal],
                        destinations: vec![host.to_owned()],
                    },
                    expires_at: None,
                    import_id,
                    alias: SecretAlias::parse(slot).expect("secret alias"),
                },
            )
            .await
            .expect("grant broker-copy secret");
        let binding_id = AgentSecretBindingId::new();
        let next_revision = AgentInstanceRevisionId::new();
        secret_service
            .bind_secret(
                actor,
                BindSecret {
                    command_key: secret_key("bind", binding_id.as_uuid()),
                    binding_id,
                    instance_id,
                    expected_revision_id: revision,
                    new_revision_id: next_revision,
                    import_id,
                    slot: SecretSlotKey::parse(slot).expect("slot"),
                    mode: DeliveryMode::Brokered,
                    phases: vec![ExecutionPhase::Normal],
                    attachment_ids: vec![attachment_id.as_uuid()],
                    destinations: vec![host.to_owned()],
                },
            )
            .await
            .unwrap_or_else(|error| panic!("bind broker-copy secret {slot}: {error:?}"));
        revision = next_revision;
        imports.insert(
            slot.to_owned(),
            (secret_id, import_id, version_id, host.to_owned()),
        );
    }
    (revision, imports)
}

pub(super) async fn declare_source_rules(
    pool: &PgPool,
    secret_service: &SecretService<LocalKeyProvider>,
    actor: &AuthenticatedIdentity,
    imports: &BTreeMap<String, (SecretId, SecretImportId, SecretVersionId, String)>,
    revision: AgentInstanceRevisionId,
    source_model_rule_id: Uuid,
    source_relay_rule_id: Uuid,
) -> Vec<(Uuid, Uuid)> {
    let mut source_rules = Vec::new();
    for (slot, (_secret_id, import_id, version_id, host)) in imports {
        let binding_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM agent_secret_bindings
             WHERE instance_revision_id = $1 AND import_id = $2 AND slot_key = $3",
        )
        .bind(revision.as_uuid())
        .bind(import_id.as_uuid())
        .bind(slot)
        .fetch_one(pool)
        .await
        .expect("active source binding");
        let rule_id = if slot == "model" {
            source_model_rule_id
        } else {
            source_relay_rule_id
        };
        secret_service
            .declare_brokered_https_rule(
                actor,
                DeclareBrokeredHttpsRule {
                    command_key: secret_key("rule", rule_id),
                    rule_id,
                    binding_id: AgentSecretBindingId::from_uuid(binding_id),
                    destination: format!("https://{host}"),
                    header: String::from("authorization"),
                    header_prefix: Some(String::from("Bearer ")),
                },
            )
            .await
            .expect("declare source rule");
        source_rules.push((rule_id, version_id.as_uuid()));
    }
    source_rules.sort_by_key(|(rule_id, _)| *rule_id);
    source_rules
}

pub(super) async fn rotate_source_secrets(
    pool: &PgPool,
    secret_service: &SecretService<LocalKeyProvider>,
    actor: &AuthenticatedIdentity,
    imports: &BTreeMap<String, (SecretId, SecretImportId, SecretVersionId, String)>,
    revision: AgentInstanceRevisionId,
) -> (Vec<BrokerRuleSnapshot>, BTreeMap<Uuid, Uuid>) {
    let source_rules_before = sqlx::query_as::<_, BrokerRuleSnapshot>(
        "SELECT id, binding_id, instance_revision_id, secret_version_id,
                destination_origin, location_kind, header_name, header_prefix,
                normalized_hash
         FROM brokered_secret_rules
         WHERE instance_revision_id = $1 ORDER BY id",
    )
    .bind(revision.as_uuid())
    .fetch_all(pool)
    .await
    .expect("capture source broker rules before update");
    assert_eq!(source_rules_before.len(), 2);
    for (secret_id, _import_id, old_version_id, _) in imports.values() {
        secret_service
            .rotate(
                actor,
                RotateSecret {
                    command_key: secret_key("rotate", secret_id.as_uuid()),
                    secret_id: *secret_id,
                    expected_active_version_id: *old_version_id,
                    new_version_id: SecretVersionId::new(),
                    value: SecretValue::new("broker-copy-rotated-value")
                        .expect("rotated secret value"),
                },
            )
            .await
            .expect("rotate active broker-copy secret");
    }
    let active_versions: BTreeMap<Uuid, Uuid> = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT rule.id, secret.active_version_id
         FROM brokered_secret_rules rule
         JOIN agent_secret_bindings binding ON binding.id = rule.binding_id
         JOIN secret_imports imported ON imported.id = binding.import_id
         JOIN secrets secret ON secret.id = imported.secret_id
         WHERE rule.instance_revision_id = $1",
    )
    .bind(revision.as_uuid())
    .fetch_all(pool)
    .await
    .expect("capture rotated active versions")
    .into_iter()
    .filter_map(|(rule_id, version_id)| version_id.map(|version| (rule_id, version)))
    .collect();
    assert_eq!(active_versions.len(), 2);
    assert!(
        source_rules_before
            .iter()
            .all(|rule| active_versions[&rule.id] != rule.secret_version_id)
    );
    (source_rules_before, active_versions)
}
