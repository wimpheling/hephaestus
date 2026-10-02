use super::*;
use release_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};
use release_postgres::ReleaseServiceError;

#[tokio::test]
#[serial]
async fn explicit_volume_slots_publish_but_legacy_import_fails_before_effects() {
    let pool = pool()
        .await
        .expect("HEPHAESTUS_POSTGRES_TEST_URL must be set for recipe integration");
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("migrations");
    let source = format!(
        "{}\n[[volume_slots]]\nslot = \"data\"\nguest_path = \"/data\"\naccess_mode = \"read_write\"\nrequired = true\nminimum_capacity_bytes = 16777216\n",
        reusable_config()
    );
    let fixture = seed_with_config(&pool, &source).await;
    let service = ReleaseService::new(
        worker_pool().await.expect("worker pool"),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let release_id = ReleaseId::new();
    let agent_id = ReleaseAgentId::new();
    publish(&service, &fixture, release_id, agent_id).await;
    let (contract, hash): (Value, Vec<u8>) = sqlx::query_as(
        "SELECT runtime_contract, runtime_contract_hash FROM release_agents WHERE id = $1",
    )
    .bind(agent_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored contract");
    let authored: Vec<VolumeSlotDeclaration> =
        serde_json::from_value(contract["volume_slots"].clone()).expect("typed slots");
    assert_eq!(
        authored,
        vec![
            VolumeSlotDeclaration::new(
                CapabilitySlotKey::parse("data").expect("key"),
                GuestMountPath::parse("/data").expect("path"),
                VolumeAccessMode::ReadWrite,
                true,
                16_777_216
            )
            .expect("slot")
        ]
    );
    assert_eq!(
        hash,
        Sha256::digest(serde_json::to_vec(&contract).expect("JSON")).to_vec()
    );
    let instance = AgentInstanceId::new();
    let actor = identity(fixture.actor);
    for _ in 0..2 {
        assert!(matches!(
            service
                .import_agent(&actor, import(&fixture, agent_id, instance))
                .await,
            Err(ReleaseServiceError::CapabilityResourceUnavailable)
        ));
        assert_no_import_effects(&pool, instance).await;
    }
    sqlx::query("DELETE FROM project_maintainers WHERE user_id = $1 AND project_id = (SELECT repository.project_id FROM releases JOIN repositories AS repository ON repository.id = releases.repository_id WHERE releases.id = $2)")
        .bind(fixture.actor.as_uuid()).bind(release_id.as_uuid()).execute(&pool).await.expect("revoke source maintainer");
    sqlx::query("DELETE FROM organization_members WHERE user_id = $1 AND organization_id = $2")
        .bind(fixture.actor.as_uuid())
        .bind(fixture.organization.as_uuid())
        .execute(&pool)
        .await
        .expect("revoke source organization membership");
    assert!(matches!(
        service
            .import_agent(&actor, import(&fixture, agent_id, instance))
            .await,
        Err(ReleaseServiceError::AuthorizationDenied)
    ));
    assert_no_import_effects(&pool, instance).await;
}

#[tokio::test]
#[serial]
async fn malformed_persisted_volume_slots_fail_closed_under_actor_context() {
    let pool = pool()
        .await
        .expect("HEPHAESTUS_POSTGRES_TEST_URL must be set for recipe integration");
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("migrations");
    let fixture = seed(&pool).await;
    let service = ReleaseService::new(
        worker_pool().await.expect("worker pool"),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let release_id = ReleaseId::new();
    let agent_id = ReleaseAgentId::new();
    publish(&service, &fixture, release_id, agent_id).await;
    let malformed_id = seed_export(&pool, release_id, agent_id, json!(null)).await;
    let instance = AgentInstanceId::new();
    assert!(matches!(
        service
            .import_agent(
                &identity(fixture.actor),
                import(&fixture, malformed_id, instance)
            )
            .await,
        Err(ReleaseServiceError::InvalidStoredData)
    ));
    assert_no_import_effects(&pool, instance).await;
}

#[tokio::test]
#[serial]
async fn legacy_instance_update_rejects_explicit_slots_without_mutation() {
    let pool = pool()
        .await
        .expect("HEPHAESTUS_POSTGRES_TEST_URL must be set for recipe integration");
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("migrations");
    let fixture = seed(&pool).await;
    let service = ReleaseService::new(
        worker_pool().await.expect("worker pool"),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let release_id = ReleaseId::new();
    let agent_id = ReleaseAgentId::new();
    publish(&service, &fixture, release_id, agent_id).await;
    let instance = AgentInstanceId::new();
    let actor = identity(fixture.actor);
    let import_command = import(&fixture, agent_id, instance);
    let revision = import_command.revision_id;
    service
        .import_agent(&actor, import_command)
        .await
        .expect("legacy import");
    let candidate = seed_export(
        &pool,
        release_id,
        agent_id,
        json!([{
            "slot": "data", "guest_path": "/data", "access_mode": "read_only",
            "required": true, "minimum_capacity_bytes": 16_777_216
        }]),
    )
    .await;
    let before = instance_state(&pool, instance).await;
    let update = AgentUpdateId::new();
    let next_revision = AgentInstanceRevisionId::new();
    let command_key = key("update-recipe-slots", update.as_uuid());
    let result = service
        .create_update(
            &actor,
            CreateInstanceUpdate {
                command_key,
                update_id: update,
                instance_id: instance,
                expected_revision_id: revision,
                candidate_revision_id: next_revision,
                candidate_release_agent_id: candidate,
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("name"),
                    ParameterValue::String("warning".to_owned()),
                )]),
                brokered_rule_copies: Vec::new(),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: "test/v1".to_owned(),
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(ReleaseServiceError::CapabilityResourceUnavailable)
    ));
    assert_eq!(instance_state(&pool, instance).await, before);
    let effects: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_updates WHERE id = $1),
                (SELECT count(*) FROM agent_instance_revisions WHERE id = $2),
                (SELECT count(*) FROM release_command_inbox WHERE command_key = $3)",
    )
    .bind(update.as_uuid())
    .bind(next_revision.as_uuid())
    .bind(command_key.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .expect("effects");
    assert_eq!(effects, (0, 0, 0));
}

async fn instance_state(
    pool: &PgPool,
    instance: AgentInstanceId,
) -> (Option<Uuid>, String, bool, i64) {
    sqlx::query_as("SELECT active_revision_id, state, run_gate_open, version FROM agent_instances WHERE id = $1")
        .bind(instance.as_uuid()).fetch_one(pool).await.expect("instance state")
}

async fn publish(
    service: &ReleaseService,
    fixture: &Fixture,
    release_id: ReleaseId,
    agent_id: ReleaseAgentId,
) {
    service
        .complete_build(CompleteBuild {
            command_key: key("complete-recipe-slots", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            release_agent_id: agent_id,
            version: ReleaseVersion::parse("recipe-slots-v1").expect("version"),
            artifacts: vec![release_test_artifact(
                "bin/reviewer",
                ArtifactKind::Executable,
                "application/octet-stream",
                1,
            )],
        })
        .await
        .expect("complete authored-slot release");
    service
        .publish(
            &identity(fixture.actor),
            key("publish-recipe-slots", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish release");
}

fn import(
    fixture: &Fixture,
    release_agent_id: ReleaseAgentId,
    instance_id: AgentInstanceId,
) -> ImportAgent {
    ImportAgent {
        command_key: key("import-recipe-slots", instance_id.as_uuid()),
        instance_id,
        revision_id: AgentInstanceRevisionId::new(),
        project_id: fixture.first_project,
        release_agent_id,
        name: InstanceName::parse("recipe-slot-consumer").expect("name"),
        parameters: BTreeMap::from([(
            ParameterName::parse("severity").expect("name"),
            ParameterValue::String("warning".to_owned()),
        )]),
        selected_policy: selected_policy(),
        platform_policy: platform_policy(),
        platform_policy_version: "test/v1".to_owned(),
    }
}

async fn assert_no_import_effects(pool: &PgPool, instance: AgentInstanceId) {
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_instances WHERE id = $1),
                (SELECT count(*) FROM agent_instance_state_volumes WHERE instance_id = $1),
                (SELECT count(*) FROM agent_instance_revisions WHERE instance_id = $1),
                (SELECT count(*) FROM release_command_inbox WHERE command_key = $2)",
    )
    .bind(instance.as_uuid())
    .bind(
        key("import-recipe-slots", instance.as_uuid())
            .as_bytes()
            .as_slice(),
    )
    .fetch_one(pool)
    .await
    .expect("inspect rejected import effects");
    assert_eq!(counts, (0, 0, 0, 0));
}

async fn seed_export(
    pool: &PgPool,
    release: ReleaseId,
    agent: ReleaseAgentId,
    authored_slots: Value,
) -> ReleaseAgentId {
    let malformed_release = ReleaseId::new();
    let malformed_agent = ReleaseAgentId::new();
    sqlx::query("INSERT INTO releases (id, repository_id, version, source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, state, published_at) SELECT $1, repository_id, 'malformed-volume-slots-v1', source_commit, source_ref, build_request_id, build_definition_hash, configuration, configuration_hash, manifest_hash, 'published', now() FROM releases WHERE id = $2")
        .bind(malformed_release.as_uuid()).bind(release.as_uuid()).execute(pool).await.expect("seed historical malformed release");
    let mut contract: Value =
        sqlx::query_scalar("SELECT runtime_contract FROM release_agents WHERE id = $1")
            .bind(agent.as_uuid())
            .fetch_one(pool)
            .await
            .expect("stored legacy contract");
    contract["volume_slots"] = authored_slots;
    let hash = Sha256::digest(serde_json::to_vec(&contract).expect("JSON"));
    sqlx::query("INSERT INTO release_agents (id, release_id, family_id, agent_key, display_name, runtime_contract, runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state) SELECT $1, $2, family_id, agent_key, display_name, $3, $4, parameter_schema, secret_slot_schema, requires_state FROM release_agents WHERE id = $5")
        .bind(malformed_agent.as_uuid()).bind(malformed_release.as_uuid()).bind(contract).bind(hash.as_slice())
        .bind(agent.as_uuid()).execute(pool).await.expect("seed historical malformed export");
    malformed_agent
}
