use super::release_rows::BuildRow;
use super::ui_publication;
use super::{
    AgentConfig, AgentFamilyId, AgentKey, ArtifactPath, CapabilitySlotKey, CompleteBuild, Digest,
    REUSABLE_RELEASE_VERSION, ReleaseId, ReleaseService, ReleaseServiceError, RuntimePolicy,
    Sha256, Uuid, Value, artifact_kind_name, artifact_manifest_hash, decode_hash, existing_command,
    insert_release_git_ceiling, json, operation_names, parameter_schema, record_command,
    release_capability_requirements, release_capability_requirements_hash, runtime_policy,
    validate_artifact,
};
use agent_config::build_identity::base_build_definition_hash;
use agent_config::ui::gateway_resolution::ReleaseAgentBinding;
use agent_config::ui::static_resolution::StaticArtifactCandidate;

impl ReleaseService {
    /// Freezes complete build provenance into a draft release. This trusted
    /// worker operation accepts only artifact metadata from a completed safe
    /// importer; it never receives repository-controlled storage paths.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for incomplete builds, invalid configuration or
    /// artifacts, conflicting publication identity, or database failure.
    // The ordered inserts document the one-transaction draft invariant.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            build_request_id = %command.build_request_id,
            release_id = %command.release_id,
            release_agent_id = %command.release_agent_id
        )
    )]
    pub async fn complete_build(
        &self,
        command: CompleteBuild,
    ) -> Result<ReleaseId, ReleaseServiceError> {
        if command.artifacts.is_empty() {
            return Err(ReleaseServiceError::IncompleteArtifacts);
        }
        let mut tx = self.pool.begin().await?;
        if let Some(id) = existing_command(&mut tx, command.command_key, "complete_build").await? {
            tx.commit().await?;
            return Ok(ReleaseId::from_uuid(id.0));
        }
        let build: BuildRow = sqlx::query_as(
            "SELECT request.repository_id, request.source_commit, request.source_ref,
                    request.build_definition_hash, selected.image_reference AS guest_image_reference,
                    request.state
             FROM build_requests AS request
             JOIN build_request_images AS selected
               ON selected.build_request_id = request.id
              AND selected.execution_context = 'guest'
             WHERE request.id = $1 FOR UPDATE OF request",
        )
        .bind(command.build_request_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if !matches!(build.state.as_str(), "importing" | "succeeded") {
            return Err(ReleaseServiceError::BuildNotImporting);
        }
        let config_row: (Value, Option<String>) = sqlx::query_as(
            "SELECT config, normalized_config_hash
             FROM agent_config_revisions
             WHERE repository_id = $1 AND commit_sha = $2
               AND schema_version = $3 AND status = 'valid'
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(build.repository_id)
        .bind(&build.source_commit)
        .bind(i32::try_from(REUSABLE_RELEASE_VERSION).unwrap_or(i32::MAX))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?;
        let config: AgentConfig = serde_json::from_value(config_row.0.clone())?;
        let guest_image_reference = build
            .guest_image_reference
            .clone()
            .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?;
        let agent_key = AgentKey::parse(
            config
                .agent
                .key
                .clone()
                .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?,
        )?;
        let base_build_definition_hash = config
            .build
            .as_ref()
            .map(base_build_definition_hash)
            .transpose()?;
        let static_artifacts = command
            .artifacts
            .iter()
            .map(|artifact| {
                validate_artifact(artifact)?;
                Ok(StaticArtifactCandidate {
                    path: artifact.path.clone(),
                    id: artifact.id,
                    kind: artifact.kind,
                    media_type: artifact.media_type.clone(),
                    size_bytes: artifact.size_bytes,
                })
            })
            .collect::<Result<Vec<_>, ReleaseServiceError>>()?;
        let release_agents = vec![ReleaseAgentBinding {
            agent_key: agent_key.clone(),
            release_agent_id: command.release_agent_id,
        }];
        let ui_publication = ui_publication::load_ui_publication(
            &mut tx,
            command.build_request_id,
            ui_publication::UiPublicationCandidates {
                repository_id: forge_domain::RepositoryId::from_uuid(build.repository_id),
                base_build_definition_hash,
                static_artifacts: &static_artifacts,
                release_agents: &release_agents,
            },
        )
        .await?;
        let family_id = AgentFamilyId::new();
        let stored_family: Uuid = sqlx::query_scalar(
            "INSERT INTO agent_families (id, repository_id, agent_key)
             VALUES ($1, $2, $3)
             ON CONFLICT (repository_id, agent_key)
             DO UPDATE SET agent_key = EXCLUDED.agent_key
             RETURNING id",
        )
        .bind(family_id.as_uuid())
        .bind(build.repository_id)
        .bind(agent_key.as_str())
        .fetch_one(&mut *tx)
        .await?;
        let family_id = AgentFamilyId::from_uuid(stored_family);
        let manifest_hash = artifact_manifest_hash(&command.artifacts);
        let configuration_hash = decode_hash(
            config_row
                .1
                .as_deref()
                .ok_or(ReleaseServiceError::ReusableConfigurationMissing)?,
        )?;
        sqlx::query(
            "INSERT INTO releases
             (id, repository_id, version, source_commit, source_ref,
              build_request_id, build_definition_hash, configuration,
              configuration_hash, manifest_hash, state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'draft')",
        )
        .bind(command.release_id.as_uuid())
        .bind(build.repository_id)
        .bind(command.version.as_str())
        .bind(&build.source_commit)
        .bind(&build.source_ref)
        .bind(command.build_request_id.as_uuid())
        .bind(&build.build_definition_hash)
        .bind(&config_row.0)
        .bind(configuration_hash.as_slice())
        .bind(manifest_hash.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        for artifact in &command.artifacts {
            sqlx::query(
                "INSERT INTO release_artifacts
                 (id, release_id, path, kind, mode, content_hash, size_bytes,
                  media_type, storage_key)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(artifact.id.as_uuid())
            .bind(command.release_id.as_uuid())
            .bind(artifact.path.as_str())
            .bind(artifact_kind_name(artifact.kind))
            .bind(i32::from(artifact.mode))
            .bind(artifact.content_hash.as_bytes().as_slice())
            .bind(
                i64::try_from(artifact.size_bytes)
                    .map_err(|_| ReleaseServiceError::InvalidArtifact)?,
            )
            .bind(&artifact.media_type)
            .bind(artifact.storage_key)
            .execute(&mut *tx)
            .await?;
        }
        let policy = runtime_policy(&config);
        let parameters = parameter_schema(&config)?;
        let capability_requirements =
            release_capability_requirements(command.release_agent_id, &config.capability_slots)?;
        let capability_requirements_hash =
            release_capability_requirements_hash(&capability_requirements);
        let executable = ArtifactPath::parse(config.guest.command.clone())?;
        let working_directory = ArtifactPath::parse(config.guest.working_directory.clone())?;
        let contract = json!({
            "executable": executable,
            "arguments": config.guest.arguments,
            "working_directory": working_directory,
            "image_reference": guest_image_reference,
            "requires_state": config.state_volume.enabled,
            "publication_mode": config.publication.mode,
            "publication_repository_slot": config
                .publication
                .repository_slot
                .as_ref()
                .map(CapabilitySlotKey::as_str),
            "policy_ceiling": policy,
            "workspace": {
                "source": "/workspace/repo",
                "work": "/workspace/work",
                "release": "/release",
                "state": "/var/lib/hephaestus",
                "parameters": "/run/hephaestus/parameters.json"
            }
        });
        let contract_bytes = serde_json::to_vec(&contract)?;
        let contract_hash: [u8; 32] = Sha256::digest(&contract_bytes).into();
        sqlx::query(
            "INSERT INTO release_agents
             (id, release_id, family_id, agent_key, display_name,
              runtime_contract, runtime_contract_hash, parameter_schema,
              secret_slot_schema, capability_requirements_hash,
              requires_state, update_hook, publication_mode,
              publication_repository_slot)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(command.release_agent_id.as_uuid())
        .bind(command.release_id.as_uuid())
        .bind(family_id.as_uuid())
        .bind(agent_key.as_str())
        .bind(&config.agent.name)
        .bind(contract)
        .bind(contract_hash.as_slice())
        .bind(serde_json::to_value(&parameters)?)
        .bind(serde_json::to_value(&config.secret_slots)?)
        .bind(capability_requirements_hash.as_slice())
        .bind(config.state_volume.enabled)
        .bind(
            config
                .update_hook
                .as_ref()
                .map(|hook| {
                    serde_json::to_value(json!({
                        "command": hook.command,
                        "arguments": hook.arguments,
                        "timeout_seconds": hook.timeout_seconds,
                        // Update-hook declarations currently specify only
                        // CPU and memory.  They inherit the release's
                        // provider-neutral network ceiling so the stored
                        // RuntimePolicy remains complete and typed.
                        "resources": RuntimePolicy {
                            vcpus: hook.resources.vcpus,
                            memory_mib: hook.resources.memory_mib,
                            network: policy.network,
                        },
                    }))
                })
                .transpose()?,
        )
        .bind(config.publication.mode.as_str())
        .bind(
            config
                .publication
                .repository_slot
                .as_ref()
                .map(CapabilitySlotKey::as_str),
        )
        .execute(&mut *tx)
        .await?;
        for (purpose, requirement) in &capability_requirements {
            sqlx::query(
                "INSERT INTO release_capability_requirements
                 (id, release_agent_id, slot_key, purpose, resource_kind,
                  required_operations, optional_operations, slot_required,
                  normalized_hash)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(requirement.id().as_uuid())
            .bind(command.release_agent_id.as_uuid())
            .bind(requirement.slot().as_str())
            .bind(purpose)
            .bind(requirement.resource_kind().as_str())
            .bind(operation_names(requirement.required_operations()))
            .bind(operation_names(requirement.optional_operations()))
            .bind(requirement.slot_required())
            .bind(requirement.normalized_hash().as_bytes().as_slice())
            .execute(&mut *tx)
            .await?;
            let declaration = config
                .capability_slots
                .iter()
                .find(|slot| slot.key == requirement.slot().as_str())
                .ok_or(ReleaseServiceError::InvalidStoredData)?;
            if let Some(ceiling) = declaration.git_ceiling()? {
                insert_release_git_ceiling(
                    &mut tx,
                    command.release_agent_id,
                    requirement.id(),
                    &ceiling,
                )
                .await?;
            }
        }
        if let Some(publication) = ui_publication.as_ref() {
            ui_publication::persist_ui_publication(
                &mut tx,
                command.release_id,
                command.build_request_id,
                publication,
            )
            .await?;
        }
        sqlx::query(
            "UPDATE build_requests SET state = 'succeeded', completed_at = now()
             WHERE id = $1",
        )
        .bind(command.build_request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "complete_build",
            command.release_id.as_uuid(),
            Some(command.release_agent_id.as_uuid()),
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(command.release_id)
    }
}
