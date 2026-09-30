use super::*;

/// Prepares the immutable release and two isolated consumer instances.
///
/// # Panics
/// Panics when a fixture operation or publication assertion fails.
// This phase intentionally keeps its setup SQL and immutable publication checks together.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn prepare() -> Option<IsolatedPublishedContext> {
    let pool = pool().await?;
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let fixture = seed(&pool).await;
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let artifact = ReleaseArtifactInput {
        id: ReleaseArtifactId::new(),
        path: ArtifactPath::parse("bin/reviewer").expect("artifact path should validate"),
        kind: ArtifactKind::Executable,
        mode: 0o555,
        content_hash: ContentHash::digest(b"built-reviewer-v1"),
        size_bytes: 17,
        media_type: String::from("application/octet-stream"),
        storage_key: Uuid::new_v4(),
    };
    let completed = service
        .complete_build(CompleteBuild {
            command_key: key("complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("v1.0.0").expect("version should validate"),
            release_agent_id,
            artifacts: vec![artifact],
        })
        .await
        .expect("complete imported build");
    assert_eq!(completed, release_id);
    let stored_publication_mode: String =
        sqlx::query_scalar("SELECT publication_mode FROM release_agents WHERE id = $1")
            .bind(release_agent_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("stored release publication mode");
    assert_eq!(stored_publication_mode, "proposal");
    let mutable_publication_mode =
        sqlx::query("UPDATE release_agents SET publication_mode = 'runtime_git' WHERE id = $1")
            .bind(release_agent_id.as_uuid())
            .execute(&pool)
            .await;
    assert!(
        mutable_publication_mode.is_err(),
        "release publication mode must be immutable"
    );
    let repeated = service
        .complete_build(CompleteBuild {
            command_key: key("complete", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("v1.0.0").expect("version should validate"),
            release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("ignored-on-retry")
                    .expect("artifact path should validate"),
                kind: ArtifactKind::File,
                mode: 0o444,
                content_hash: ContentHash::digest(b"ignored"),
                size_bytes: 7,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("duplicate completion should return durable release");
    assert_eq!(repeated, release_id);

    let actor = identity(fixture.actor);
    service
        .publish(&actor, key("publish", release_id.as_uuid()), release_id)
        .await
        .expect("source maintainer should publish");
    let immutable = sqlx::query("UPDATE releases SET source_commit = $2 WHERE id = $1")
        .bind(release_id.as_uuid())
        .bind("b".repeat(40))
        .execute(&pool)
        .await;
    assert!(immutable.is_err(), "published provenance must be immutable");

    let first_instance = AgentInstanceId::new();
    let first_revision = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-first", first_instance.as_uuid()),
                instance_id: first_instance,
                revision_id: first_revision,
                project_id: fixture.first_project,
                release_agent_id,
                name: InstanceName::parse("reviewer").expect("name should validate"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("first project should import");
    let revision_publication_mode: String =
        sqlx::query_scalar("SELECT publication_mode FROM agent_instance_revisions WHERE id = $1")
            .bind(first_revision.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("stored revision publication mode");
    assert_eq!(revision_publication_mode, "proposal");
    let second_instance = AgentInstanceId::new();
    let second_revision = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-second", second_instance.as_uuid()),
                instance_id: second_instance,
                revision_id: second_revision,
                project_id: fixture.second_project,
                release_agent_id,
                name: InstanceName::parse("reviewer").expect("name should validate"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter should validate"),
                    ParameterValue::String(String::from("error")),
                )]),
                selected_policy: RuntimePolicy {
                    vcpus: 1,
                    memory_mib: 512,
                    network: NetworkAccess::Disabled,
                },
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("second project should independently import");

    let instances: Vec<(Uuid, Uuid, Uuid, bool)> = sqlx::query_as(
        "SELECT instance.id, instance.project_id, instance.state_volume_id,
                revision.runnable
         FROM agent_instances AS instance
         JOIN agent_instance_revisions AS revision
           ON revision.id = instance.active_revision_id
         WHERE instance.id = ANY($1)
         ORDER BY instance.id",
    )
    .bind(vec![first_instance.as_uuid(), second_instance.as_uuid()])
    .fetch_all(&pool)
    .await
    .expect("stored isolated instances");
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].1, instances[1].1);
    assert_ne!(instances[0].2, instances[1].2);
    assert!(
        instances.iter().all(|row| !row.3),
        "required symbolic secret slot should be visibly unrunnable"
    );
    let consumer_runtime_override_columns: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'agent_instance_revisions'
           AND column_name = ANY($1)",
    )
    .bind(vec![
        "command",
        "arguments",
        "working_directory",
        "image_reference",
        "mounts",
        "requires_state",
    ])
    .fetch_one(&pool)
    .await
    .expect("inspect consumer revision columns");
    assert_eq!(
        consumer_runtime_override_columns, 0,
        "consumer revisions must reference immutable release-owned runtime fields"
    );

    Some(IsolatedPublishedContext {
        pool,
        fixture,
        service,
        actor,
        release_id,
        release_agent_id,
        first_instance,
        first_revision,
        second_instance,
    })
}
