use super::*;

#[tokio::test]
#[serial]
async fn runtime_git_mode_is_frozen_into_release_and_instance_revision() {
    let Some(pool) = pool().await else {
        return;
    };
    sqlx::migrate!("../../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let config = reusable_config()
        .replace("mount = true", "mount = false")
        .replace(
            "[state_volume]",
            "[publication]\nmode = \"runtime_git\"\nrepository_slot = \"content\"\n\n\
             [[capability_slots]]\nkey = \"content\"\npurpose = \"Publish content\"\n\
             resource_kind = \"repository\"\nrequired_operations = [\"git_read\"]\n\
             optional_operations = [\"update_ref\"]\nrequired = true\n\n\
             [capability_slots.git]\nref_globs = [\"refs/heads/content\"]\n\
             changed_path_globs = [\"content/**\"]\nexact_parent_required = false\n\
             transfer = { request_bytes = 1048576, pack_bytes = 8388608, object_count = 10000, ref_updates = 8 }\n\n\
             [state_volume]",
        );
    let git_authority = parse(config.as_bytes())
        .config
        .expect("valid runtime Git config")
        .capability_slots
        .into_iter()
        .find(|slot| slot.key == "content")
        .expect("content slot")
        .git_ceiling()
        .expect("valid Git ceiling")
        .expect("typed Git ceiling");
    let fixture = seed_with_config(&pool, &config).await;
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    service
        .complete_build(CompleteBuild {
            command_key: key("complete-runtime-git", release_id.as_uuid()),
            build_request_id: fixture.build,
            release_id,
            version: ReleaseVersion::parse("runtime-git-v1").expect("release version"),
            release_agent_id,
            artifacts: vec![ReleaseArtifactInput {
                id: ReleaseArtifactId::new(),
                path: ArtifactPath::parse("bin/reviewer").expect("artifact path"),
                kind: ArtifactKind::Executable,
                mode: 0o555,
                content_hash: ContentHash::digest(b"runtime-git-reviewer"),
                size_bytes: 20,
                media_type: String::from("application/octet-stream"),
                storage_key: Uuid::new_v4(),
            }],
        })
        .await
        .expect("complete runtime Git release");
    let actor = identity(fixture.actor);
    service
        .publish(
            &actor,
            key("publish-runtime-git", release_id.as_uuid()),
            release_id,
        )
        .await
        .expect("publish runtime Git release");
    let instance_id = AgentInstanceId::new();
    let revision_id = AgentInstanceRevisionId::new();
    service
        .import_agent(
            &actor,
            ImportAgent {
                command_key: key("import-runtime-git", instance_id.as_uuid()),
                instance_id,
                revision_id,
                project_id: fixture.first_project,
                release_agent_id,
                name: InstanceName::parse("runtime-git-reviewer").expect("instance name"),
                parameters: BTreeMap::from([(
                    ParameterName::parse("severity").expect("parameter name"),
                    ParameterValue::String(String::from("warning")),
                )]),
                selected_policy: selected_policy(),
                platform_policy: platform_policy(),
                platform_policy_version: String::from("platform/v1"),
            },
        )
        .await
        .expect("import runtime Git instance");
    sqlx::query(
        "INSERT INTO project_capability_granters (project_id, user_id, created_by)
         VALUES ($1, $2, $2)",
    )
    .bind(fixture.first_project.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(&pool)
    .await
    .expect("grant capability delegation role");
    let bound_revision = AgentInstanceRevisionId::new();
    let publication_binding = CapabilityBindingId::new();
    service
        .revise_instance_capabilities(
            &actor,
            ReviseInstanceCapabilities {
                command_key: key("bind-runtime-git", bound_revision.as_uuid()),
                instance_id,
                expected_revision_id: revision_id,
                new_revision_id: bound_revision,
                bindings: vec![release_postgres::CapabilityBindingSelection {
                    binding_id: publication_binding,
                    slot: CapabilitySlotKey::parse("content").expect("capability slot"),
                    resource: CapabilityResource::new(
                        CapabilityResourceKind::Repository,
                        fixture.first_aux_repository.as_uuid(),
                    ),
                    granted_operations: vec![
                        CapabilityOperation::GitRead,
                        CapabilityOperation::UpdateRef,
                    ],
                    git_authority: Some(git_authority),
                }],
                authorization_model_version: String::from("melange/v2"),
            },
        )
        .await
        .expect("bind runtime Git repository");
    let trigger_attachment = AgentAttachmentId::new();
    service
        .create_attachment(
            &actor,
            CreateAttachment {
                command_key: key("attach-distinct-trigger", trigger_attachment.as_uuid()),
                attachment_id: trigger_attachment,
                instance_id,
                repository_id: fixture.first_repository,
                ref_selector: RefSelector::parse("refs/heads/main").expect("trigger ref"),
                trigger_policy: TriggerPolicy::Push,
            },
        )
        .await
        .expect("create distinct trigger attachment");
    let modes: (String, String, String, Uuid, Uuid) = sqlx::query_as(
        "SELECT release_agent.publication_mode, revision.publication_mode,
                release_agent.publication_repository_slot,
                revision.publication_repository_binding_id,
                binding.resource_id
         FROM release_agents AS release_agent
         JOIN agent_instance_revisions AS revision
           ON revision.release_agent_id = release_agent.id
         JOIN agent_capability_bindings AS binding
           ON binding.id = revision.publication_repository_binding_id
         WHERE release_agent.id = $1 AND revision.id = $2",
    )
    .bind(release_agent_id.as_uuid())
    .bind(bound_revision.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("immutable publication modes");
    assert_eq!(
        modes,
        (
            String::from("runtime_git"),
            String::from("runtime_git"),
            String::from("content"),
            publication_binding.as_uuid(),
            fixture.first_aux_repository.as_uuid(),
        )
    );
    let git_scope: (i16, Vec<String>, Vec<String>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT ceiling.grammar_version, binding.ref_globs,
                binding.changed_path_globs, ceiling.normalized_hash,
                binding.normalized_hash
         FROM release_git_capability_ceilings AS ceiling
         JOIN agent_git_capability_bindings AS binding
           ON binding.requirement_id = ceiling.requirement_id
         WHERE binding.binding_id = $1",
    )
    .bind(publication_binding.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("typed Git ceiling and binding");
    assert_eq!(git_scope.0, 1);
    assert_eq!(git_scope.1, vec![String::from("refs/heads/content")]);
    assert_eq!(git_scope.2, vec![String::from("content/**")]);
    assert_eq!(git_scope.3.len(), 32);
    assert_eq!(git_scope.4.len(), 32);
    assert_ne!(git_scope.3, git_scope.4, "binding hash includes repository");
    let attachment_repository: Uuid =
        sqlx::query_scalar("SELECT repository_id FROM agent_attachments WHERE id = $1")
            .bind(trigger_attachment.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("trigger attachment provenance");
    assert_ne!(attachment_repository, modes.4);
}
