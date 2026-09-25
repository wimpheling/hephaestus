//! `PostgreSQL` release and instance command adapter.

use agent_config::{
    AgentConfig, NetworkProfile, ParameterDefault, REUSABLE_RELEASE_VERSION,
    build_identity::base_build_definition_hash,
    ui::{gateway_resolution::ReleaseAgentBinding, static_resolution::StaticArtifactCandidate},
};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityError, CapabilityOperation,
    CapabilityRequirement, CapabilityRequirementId, CapabilityResource, CapabilityResourceKind,
    CapabilitySlotKey,
};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy,
    RefUpdatePolicy, RepositoryId as GitRepositoryId, TransferLimits,
};
use identity_domain::AuthenticatedIdentity;
use release_artifact_helpers::{
    artifact_kind_name, artifact_manifest_hash, parameter_schema, policy_from_contract,
    runtime_policy, validate_artifact,
};
use release_brokered_helpers::{
    candidate_accepts_binding, clone_brokered_secret_rule, required_slot_diagnostics,
    update_contract_diagnostics, validate_brokered_rule_copies,
};
use release_capability_binding::{
    capability_diagnostics_from_json, insert_capability_binding, insert_git_capability_binding,
    insert_release_git_ceiling, operation_names, release_capability_requirements,
    release_capability_requirements_hash,
};
use release_capability_helpers::{
    git_authority_matches_capability, load_capability_requirements,
    load_carried_capability_bindings, load_git_ceilings, stored_requirement,
};
use release_capability_selection::{
    authorize_capability_selection, capability_resource_is_in_project,
};
use release_command_events::{
    append_event, append_instance_event, decode_hash, existing_command,
    is_update_admission_generation_conflict, record_command, ref_selector_string,
    trigger_policy_name,
};
use release_domain::{
    AgentAttachmentId, AgentFamilyId, AgentInstanceId, AgentInstanceRevisionId, AgentKey,
    AgentUpdateId, ArtifactKind, ArtifactPath, ContentHash, NetworkAccess, ParameterDeclaration,
    ParameterDocument, ParameterName, ParameterType, ParameterValue, RefSelector, ReleaseAgentId,
    ReleaseCommandKey, ReleaseId, RuntimePolicy,
};
use release_revision_persistence::{
    clone_revision_binding, insert_revision, insert_revision_with_binding_ids,
    insert_update_candidate_revision,
};
use release_rows::{
    BrokeredRuleCloneRow, BuildRow, CapabilityRequirementRow, CapabilityRevisionSourceRow,
    CarriedCapabilityBinding, DeferredMaterializationRow, GitCapabilityRow, ReleaseAgentRow,
    RevisionBindingRow, RevisionUpdateRow, StoredCapabilityBindingRow, UpdateCandidateRow,
    UpdateCurrentRow, UpdateHookAdmissionRow, UpdateLifecycleRow, UpdateRecoveryRow,
    UpdateRunResultRow, append_rejected_update_event, append_uncertain_update_events,
};
pub use release_service::*;
use release_update_gate::{
    enqueue_mailbox_wakes, existing_terminal_decision, mark_volume_lease,
    materialize_deferred_triggers, recovery_decision, reopen_after_update,
};
use run_domain::{RunKind, StartRun};
use runtime_types::{CommandId, RunId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use time::OffsetDateTime;
pub use ui_browser::PgUiBrowserSessionStore;
pub use ui_browser_resources::{PgUiBrowserServingStore, PgUiGenerationHostResolver};
pub use ui_installation_navigation::PgUiInstallationNavigator;
pub use ui_request_audit::{
    PgUiRequestAuditRepository, append_in_transaction as append_ui_request_audit_in_transaction,
};
use uuid::Uuid;

mod release_artifact_helpers;
mod release_brokered_helpers;
mod release_capability_binding;
mod release_capability_helpers;
mod release_capability_selection;
mod release_command_events;
mod release_revision_persistence;
mod release_rows;
mod release_update_gate;
mod ui_browser;
mod ui_browser_resources;
mod ui_installation;
mod ui_installation_navigation;
mod ui_publication;
mod ui_request_audit;

const RUN_START_SUBJECT: &str = "hephaestus.run.start";
const MAILBOX_WAKE_SUBJECT: &str = "heph.mailbox.v1.wake";
const MAILBOX_WAKE_EVENT_TYPE: &str = "mailbox.wake.v1";

/// PostgreSQL-backed release and instance command service.
pub struct ReleaseService {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

impl ReleaseService {
    /// Creates the service.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }

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

    /// Explicitly publishes and freezes one complete draft release.
    ///
    /// # Errors
    ///
    /// Fails for denial, missing/incomplete draft, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(actor_id = %identity.user_id, request_id = %identity.request_id, %release_id)
    )]
    pub async fn publish(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: ReleaseCommandKey,
        release_id: ReleaseId,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanPublish,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "publish")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query(
            "UPDATE releases SET state = 'published',
                    publication_actor_id = $2, published_at = now()
             WHERE id = $1 AND state = 'draft'
               AND EXISTS (
                    SELECT 1 FROM release_artifacts WHERE release_id = $1
               )
               AND EXISTS (
                    SELECT 1 FROM release_agents WHERE release_id = $1
               )",
        )
        .bind(release_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::Unavailable);
        }
        record_command(
            &mut tx,
            command_key,
            "publish",
            release_id.as_uuid(),
            None,
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            release_id.as_uuid(),
            "hephaestus.release.published.v1",
            "release.published.v1",
            json!({"schema_version": 1, "release_id": release_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Revokes a published release without deleting immutable provenance.
    ///
    /// Historical instances, revisions, runs, results, and artifacts retain
    /// their exact foreign-key targets, while new imports and guest starts
    /// reject the no-longer-published release.
    ///
    /// # Errors
    ///
    /// Fails for denial, a non-published release, idempotency conflict, or a
    /// database error.
    #[tracing::instrument(
        skip_all,
        fields(actor_id = %identity.user_id, request_id = %identity.request_id, %release_id)
    )]
    pub async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: ReleaseCommandKey,
        release_id: ReleaseId,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRevoke,
            ObjectRef::new(ObjectType::Release, release_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "revoke")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let changed = sqlx::query(
            "UPDATE releases
             SET state = 'revoked', revoked_at = now()
             WHERE id = $1 AND state = 'published'",
        )
        .bind(release_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::Unavailable);
        }
        record_command(
            &mut tx,
            command_key,
            "revoke",
            release_id.as_uuid(),
            None,
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            release_id.as_uuid(),
            "hephaestus.release.revoked.v1",
            "release.revoked.v1",
            json!({"release_id": release_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Imports a published release export as a project-owned instance and
    /// atomically activates its first immutable revision.
    ///
    /// Required unresolved secret slots make the revision visibly unrunnable;
    /// no secret value or tenant secret identifier is present in the release.
    ///
    /// # Errors
    ///
    /// Fails for either project-side or source-side denial, invalid parameters
    /// or policy, unpublished release, idempotency conflict, or database error.
    // Instance, optional state-volume metadata, and revision must commit as one.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            instance_id = %command.instance_id,
            release_agent_id = %command.release_agent_id,
            project_id = %command.project_id
        )
    )]
    pub async fn import_agent(
        &self,
        identity: &AuthenticatedIdentity,
        command: ImportAgent,
    ) -> Result<AgentInstanceId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, command.project_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, command.release_agent_id.as_uuid()),
        )
        .await?;
        if let Some(id) = existing_command(&mut tx, command.command_key, "import_agent").await? {
            tx.commit().await?;
            return Ok(AgentInstanceId::from_uuid(id.0));
        }
        let release: ReleaseAgentRow = sqlx::query_as(
            "SELECT agent.family_id, agent.parameter_schema,
                    agent.secret_slot_schema, agent.runtime_contract,
                    agent.requires_state, agent.publication_mode
             FROM release_agents AS agent
             JOIN releases ON releases.id = agent.release_id
             WHERE agent.id = $1 AND releases.state = 'published'",
        )
        .bind(command.release_agent_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(release.parameter_schema)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let release_policy = policy_from_contract(&release.runtime_contract)?;
        let effective_policy = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let mut diagnostics = release
            .secret_slot_schema
            .as_array()
            .ok_or(ReleaseServiceError::InvalidStoredData)?
            .iter()
            .filter(|slot| slot.get("required").and_then(Value::as_bool) == Some(true))
            .filter_map(|slot| slot.get("key").and_then(Value::as_str))
            .map(|slot| {
                json!({
                    "code": "required_secret_binding_missing",
                    "field": format!("secret_slots.{slot}")
                })
            })
            .collect::<Vec<_>>();
        let required_capability_slots: Vec<String> = sqlx::query_scalar(
            "SELECT slot_key FROM release_capability_requirements
             WHERE release_agent_id = $1 AND slot_required
             ORDER BY slot_key",
        )
        .bind(command.release_agent_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        diagnostics.extend(required_capability_slots.into_iter().map(|slot| {
            json!({
                "code": "required_capability_binding_missing",
                "field": format!("capability_slots.{slot}")
            })
        }));
        let runnable = diagnostics.is_empty();
        let state_volume_id = release.requires_state.then(Uuid::new_v4);
        sqlx::query(
            "INSERT INTO agent_instances
             (id, project_id, family_id, name, state, active_revision_id,
              state_volume_id, created_by)
             VALUES ($1, $2, $3, $4, 'active', NULL, $5, $6)",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.project_id.as_uuid())
        .bind(release.family_id)
        .bind(command.name.as_str())
        .bind(state_volume_id)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if let Some(volume_id) = state_volume_id {
            sqlx::query(
                "INSERT INTO agent_instance_state_volumes
                 (id, instance_id, state, capacity_bytes)
                 VALUES ($1, $2, 'uninitialized', 1073741824)",
            )
            .bind(volume_id)
            .bind(command.instance_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        }
        insert_revision(
            &mut tx,
            command.revision_id,
            command.instance_id,
            command.release_agent_id,
            &parameters,
            &command.selected_policy,
            &effective_policy,
            &command.platform_policy_version,
            &release.publication_mode,
            None,
            runnable,
            &diagnostics,
            identity,
        )
        .await?;
        sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $2, updated_at = now(), version = version + 1
             WHERE id = $1 AND active_revision_id IS NULL",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "import_agent",
            command.instance_id.as_uuid(),
            Some(command.revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.revision_id),
            "instance.created",
            identity,
            json!({"runnable": runnable}),
        )
        .await?;
        append_event(
            &mut tx,
            command.instance_id.as_uuid(),
            "hephaestus.agent_instance.created.v1",
            "agent_instance.created.v1",
            json!({
                "schema_version": 1,
                "instance_id": command.instance_id,
                "revision_id": command.revision_id,
                "release_agent_id": command.release_agent_id,
                "project_id": command.project_id,
                "runnable": runnable,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.instance_id)
    }

    /// Creates an exact project-bound attachment. Trigger policy lives on this
    /// record, not on the release's source branch.
    ///
    /// # Errors
    ///
    /// Fails for either instance or repository denial, cross-project target,
    /// idempotency conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id,
            instance_id = %command.instance_id,
            repository_id = %command.repository_id
        )
    )]
    pub async fn create_attachment(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateAttachment,
    ) -> Result<AgentAttachmentId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanWrite,
            ObjectRef::new(ObjectType::Repository, command.repository_id.as_uuid()),
        )
        .await?;
        if let Some(id) =
            existing_command(&mut tx, command.command_key, "create_attachment").await?
        {
            tx.commit().await?;
            return Ok(AgentAttachmentId::from_uuid(id.0));
        }
        let project_id: Uuid =
            sqlx::query_scalar("SELECT project_id FROM agent_instances WHERE id = $1")
                .bind(command.instance_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ReleaseServiceError::Unavailable)?;
        sqlx::query(
            "INSERT INTO agent_attachments
             (id, instance_id, project_id, repository_id, ref_selector,
              trigger_policy, enabled, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, true, $7)",
        )
        .bind(command.attachment_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(project_id)
        .bind(command.repository_id.as_uuid())
        .bind(ref_selector_string(&command.ref_selector))
        .bind(trigger_policy_name(command.trigger_policy))
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "create_attachment",
            command.attachment_id.as_uuid(),
            Some(command.instance_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            None,
            "attachment.created",
            identity,
            json!({
                "attachment_id": command.attachment_id,
                "repository_id": command.repository_id,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": command.instance_id,
                "attachment_id": command.attachment_id,
                "repository_id": command.repository_id,
                "action": "created",
                "enabled": true,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.attachment_id)
    }

    /// Enables or disables future triggers for an attachment.
    ///
    /// # Errors
    ///
    /// Fails for denial, a removed/missing attachment, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id
        )
    )]
    pub async fn set_attachment_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        command: SetAttachmentEnabled,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentAttachment, command.attachment_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "set_attachment_enabled")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let instance_id: Uuid = sqlx::query_scalar(
            "UPDATE agent_attachments
             SET enabled = $2, updated_at = now()
             WHERE id = $1 AND removed_at IS NULL
             RETURNING instance_id",
        )
        .bind(command.attachment_id.as_uuid())
        .bind(command.enabled)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command.command_key,
            "set_attachment_enabled",
            command.attachment_id.as_uuid(),
            Some(instance_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(instance_id),
            None,
            if command.enabled {
                "attachment.enabled"
            } else {
                "attachment.disabled"
            },
            identity,
            json!({"attachment_id": command.attachment_id}),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": instance_id,
                "attachment_id": command.attachment_id,
                "action": if command.enabled { "enabled" } else { "disabled" },
                "enabled": command.enabled,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Tombstones an attachment while preserving historical run references.
    ///
    /// # Errors
    ///
    /// Fails for denial, a missing attachment, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id
        )
    )]
    pub async fn remove_attachment(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveAttachment,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentAttachment, command.attachment_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "remove_attachment")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let instance_id: Uuid = sqlx::query_scalar(
            "UPDATE agent_attachments
             SET enabled = false, removed_at = COALESCE(removed_at, now()),
                 updated_at = now()
             WHERE id = $1
             RETURNING instance_id",
        )
        .bind(command.attachment_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command.command_key,
            "remove_attachment",
            command.attachment_id.as_uuid(),
            Some(instance_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(instance_id),
            None,
            "attachment.removed",
            identity,
            json!({"attachment_id": command.attachment_id}),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": instance_id,
                "attachment_id": command.attachment_id,
                "action": "removed",
                "enabled": false,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Creates and compare-and-swap activates a new immutable revision whose
    /// complete capability binding set is selected explicitly.
    ///
    /// Existing parameter, secret, resource, and runtime-policy fields are
    /// cloned byte-for-byte from the expected revision. Capability bindings
    /// are revalidated against the immutable release requirements and receive
    /// new revision ownership; the previous revision remains unchanged.
    ///
    /// # Errors
    ///
    /// Fails for denial, a stale revision, malformed or broadened bindings,
    /// unavailable/cross-project resources, idempotency conflict, corrupt
    /// stored requirements, or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            instance_id = %command.instance_id,
            revision_id = %command.new_revision_id
        )
    )]
    pub async fn revise_instance_capabilities(
        &self,
        identity: &AuthenticatedIdentity,
        command: ReviseInstanceCapabilities,
    ) -> Result<CapabilityRevisionResult, ReleaseServiceError> {
        if command.authorization_model_version.is_empty()
            || command.authorization_model_version.len() > 128
        {
            return Err(ReleaseServiceError::InvalidCapabilityBinding);
        }
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        if let Some((_instance, revision)) =
            existing_command(&mut tx, command.command_key, "revise_instance_capabilities").await?
        {
            let revision_id = AgentInstanceRevisionId::from_uuid(
                revision.ok_or(ReleaseServiceError::InvalidStoredData)?,
            );
            let stored: (bool, Value) = sqlx::query_as(
                "SELECT runnable, diagnostics
                 FROM agent_instance_revisions WHERE id = $1",
            )
            .bind(revision_id.as_uuid())
            .fetch_one(&mut *tx)
            .await?;
            let diagnostics = capability_diagnostics_from_json(&stored.1)?;
            tx.commit().await?;
            return Ok(CapabilityRevisionResult {
                revision_id,
                runnable: stored.0,
                diagnostics,
            });
        }
        let source: CapabilityRevisionSourceRow = sqlx::query_as(
            "SELECT instance.active_revision_id, instance.project_id,
                    revision.release_agent_id, revision.secret_bindings,
                    revision.diagnostics, agent.publication_repository_slot
             FROM agent_instances AS instance
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS agent
               ON agent.id = revision.release_agent_id
             JOIN releases AS release ON release.id = agent.release_id
             WHERE instance.id = $1
               AND instance.state IN ('active', 'update_rejected')
               AND release.state = 'published'
             FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if source.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        let carried_secrets: Vec<RevisionBindingRow> = sqlx::query_as(
            "SELECT binding.id, binding.import_id, binding.slot_key,
                    binding.delivery_mode, binding.phases,
                    binding.attachment_ids, binding.destinations,
                    binding.effective_policy, binding.effective_policy_hash
             FROM agent_secret_bindings AS binding
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE binding.instance_revision_id = $1
               AND binding.status = 'active'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND secret.status = 'active'
               AND (source_grant.expires_at IS NULL
                    OR source_grant.expires_at > now())
             ORDER BY binding.slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let expected_secret_ids: Vec<Uuid> = serde_json::from_value(source.secret_bindings)?;
        if carried_secrets.len() != expected_secret_ids.len() {
            return Err(ReleaseServiceError::SecretBindingUnavailable);
        }
        for binding in &carried_secrets {
            self.require(
                &mut tx,
                identity,
                if binding.delivery_mode == "raw" {
                    Permission::BindRaw
                } else {
                    Permission::BindBrokered
                },
                ObjectRef::new(ObjectType::SecretImport, binding.import_id),
            )
            .await?;
        }
        let new_secret_ids = carried_secrets
            .iter()
            .map(|_| Uuid::new_v4())
            .collect::<Vec<_>>();
        let rows: Vec<CapabilityRequirementRow> = sqlx::query_as(
            "SELECT id, slot_key, resource_kind, required_operations,
                    optional_operations, slot_required, normalized_hash
             FROM release_capability_requirements
             WHERE release_agent_id = $1
             ORDER BY slot_key, id",
        )
        .bind(source.release_agent_id)
        .fetch_all(&mut *tx)
        .await?;
        let requirements = rows
            .into_iter()
            .map(stored_requirement)
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let git_ceilings = load_git_ceilings(&mut tx, source.release_agent_id).await?;
        let mut selected = BTreeMap::new();
        let mut binding_ids = BTreeSet::new();
        for selection in &command.bindings {
            if !binding_ids.insert(selection.binding_id)
                || selected.insert(selection.slot.clone(), selection).is_some()
            {
                return Err(ReleaseServiceError::InvalidCapabilityBinding);
            }
        }
        let mut bindings = Vec::with_capacity(selected.len());
        let mut git_bindings = BTreeMap::new();
        for (slot, selection) in selected {
            let requirement = requirements
                .get(&slot)
                .ok_or(ReleaseServiceError::InvalidCapabilityBinding)?;
            let binding = CapabilityBinding::bind(
                selection.binding_id,
                requirement,
                selection.resource,
                selection.granted_operations.iter().copied(),
            )?;
            match (
                git_ceilings.get(&requirement.id()),
                &selection.git_authority,
            ) {
                (Some(ceiling), selected) => {
                    let authority = selected.as_ref().unwrap_or(ceiling);
                    if !git_authority_matches_capability(authority, &binding) {
                        return Err(ReleaseServiceError::InvalidCapabilityBinding);
                    }
                    let git_binding = BoundGitCapability::new(
                        GitRepositoryId::new(binding.resource().id),
                        authority.clone(),
                        ceiling,
                    )?;
                    git_bindings.insert(binding.id(), git_binding);
                }
                (None, None) => {}
                (None, Some(_)) => {
                    return Err(ReleaseServiceError::InvalidCapabilityBinding);
                }
            }
            if !capability_resource_is_in_project(
                &mut tx,
                source.project_id,
                binding.resource().kind,
                binding.resource().id,
            )
            .await?
            {
                return Err(ReleaseServiceError::CapabilityResourceUnavailable);
            }
            authorize_capability_selection(self, &mut tx, identity, &binding).await?;
            bindings.push(binding);
        }
        let missing = requirements
            .values()
            .filter(|requirement| {
                requirement.slot_required()
                    && !bindings
                        .iter()
                        .any(|binding| binding.slot() == requirement.slot())
            })
            .map(|requirement| CapabilityRevisionDiagnostic {
                code: String::from("required_capability_binding_missing"),
                slot: requirement.slot().clone(),
            })
            .collect::<Vec<_>>();
        let mut diagnostics = source
            .diagnostics
            .as_array()
            .ok_or(ReleaseServiceError::InvalidStoredData)?
            .iter()
            .filter(|value| {
                value.get("code").and_then(Value::as_str)
                    != Some("required_capability_binding_missing")
            })
            .cloned()
            .collect::<Vec<_>>();
        diagnostics.extend(missing.iter().map(|diagnostic| {
            json!({
                "code": diagnostic.code,
                "field": format!("capability_slots.{}", diagnostic.slot),
            })
        }));
        let runnable = diagnostics.is_empty();
        let publication_repository_binding_id = source
            .publication_repository_slot
            .as_deref()
            .and_then(|slot| {
                bindings
                    .iter()
                    .find(|binding| binding.slot().as_str() == slot)
            })
            .map(|binding| binding.id().as_uuid());
        let inserted = sqlx::query(
            "INSERT INTO agent_instance_revisions
             (id, instance_id, release_agent_id, parameters, parameter_hash,
              secret_bindings, resource_selection, network_restriction,
              effective_runtime_policy, effective_policy_hash,
              platform_policy_version, publication_mode,
              publication_repository_binding_id, runnable, diagnostics, created_by)
             SELECT $1, instance_id, release_agent_id, parameters,
                    parameter_hash, $7, resource_selection,
                    network_restriction, effective_runtime_policy,
                    effective_policy_hash, platform_policy_version,
                    publication_mode, $8, $3, $4, $5
             FROM agent_instance_revisions
             WHERE id = $2 AND instance_id = $6",
        )
        .bind(command.new_revision_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(runnable)
        .bind(serde_json::to_value(&diagnostics)?)
        .bind(identity.user_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(serde_json::to_value(&new_secret_ids)?)
        .bind(publication_repository_binding_id)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        for binding in &bindings {
            insert_capability_binding(
                &mut tx,
                command.new_revision_id,
                source.release_agent_id,
                binding,
                &command.authorization_model_version,
                identity,
            )
            .await?;
            if let Some(git_binding) = git_bindings.get(&binding.id()) {
                insert_git_capability_binding(
                    &mut tx,
                    command.new_revision_id,
                    binding,
                    git_binding,
                )
                .await?;
            }
        }
        for (binding, binding_id) in carried_secrets.iter().zip(&new_secret_ids) {
            clone_revision_binding(
                &mut tx,
                *binding_id,
                command.new_revision_id,
                binding,
                identity,
            )
            .await?;
        }
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state IN ('active', 'update_rejected')",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        record_command(
            &mut tx,
            command.command_key,
            "revise_instance_capabilities",
            command.instance_id.as_uuid(),
            Some(command.new_revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.new_revision_id),
            "instance.capabilities_revised",
            identity,
            json!({
                "runnable": runnable,
                "binding_count": bindings.len(),
                "authorization_model_version": command.authorization_model_version,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(CapabilityRevisionResult {
            revision_id: command.new_revision_id,
            runnable,
            diagnostics: missing,
        })
    }

    /// Validates, creates, and CAS-activates a new immutable instance
    /// revision. Existing secret bindings are cloned to new revision-bound
    /// identities only while their live imports remain bindable.
    ///
    /// # Errors
    ///
    /// Fails for denial, stale active revision, invalid typed parameters or
    /// policy, revoked secret authority, idempotency conflict, or database
    /// failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            instance_id = %command.instance_id,
            revision_id = %command.new_revision_id
        )
    )]
    pub async fn revise_instance(
        &self,
        identity: &AuthenticatedIdentity,
        command: ReviseInstance,
    ) -> Result<AgentInstanceRevisionId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        if let Some((_instance, revision)) =
            existing_command(&mut tx, command.command_key, "revise_instance").await?
        {
            tx.commit().await?;
            return Ok(AgentInstanceRevisionId::from_uuid(
                revision.ok_or(ReleaseServiceError::InvalidStoredData)?,
            ));
        }
        let current: RevisionUpdateRow = sqlx::query_as(
            "SELECT instance.active_revision_id, instance.project_id,
                    revision.release_agent_id,
                    revision.secret_bindings, revision.publication_mode,
                    agent.publication_repository_slot,
                    agent.parameter_schema,
                    agent.secret_slot_schema, agent.runtime_contract
             FROM agent_instances AS instance
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS agent
               ON agent.id = revision.release_agent_id
             JOIN releases AS release ON release.id = agent.release_id
             WHERE instance.id = $1
               AND instance.state IN ('active', 'update_rejected')
               AND release.state = 'published'
             FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if current.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(current.parameter_schema)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let release_policy = policy_from_contract(&current.runtime_contract)?;
        let effective = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let carried: Vec<RevisionBindingRow> = sqlx::query_as(
            "SELECT binding.id, binding.import_id, binding.slot_key,
                    binding.delivery_mode, binding.phases,
                    binding.attachment_ids, binding.destinations,
                    binding.effective_policy, binding.effective_policy_hash
             FROM agent_secret_bindings AS binding
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE binding.instance_revision_id = $1
               AND binding.status = 'active'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND secret.status = 'active'
               AND (source_grant.expires_at IS NULL
                    OR source_grant.expires_at > now())
             ORDER BY binding.slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let expected: Vec<Uuid> = serde_json::from_value(current.secret_bindings)?;
        if carried.len() != expected.len() {
            return Err(ReleaseServiceError::SecretBindingUnavailable);
        }
        for binding in &carried {
            self.require(
                &mut tx,
                identity,
                if binding.delivery_mode == "raw" {
                    Permission::BindRaw
                } else {
                    Permission::BindBrokered
                },
                ObjectRef::new(ObjectType::SecretImport, binding.import_id),
            )
            .await?;
        }
        let new_binding_ids = carried.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let bound_slots = carried
            .iter()
            .map(|binding| binding.slot_key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut diagnostics = required_slot_diagnostics(&current.secret_slot_schema, &bound_slots)?;
        let carried_capabilities =
            load_carried_capability_bindings(&mut tx, command.expected_revision_id).await?;
        for carried_capability in &carried_capabilities {
            if !capability_resource_is_in_project(
                &mut tx,
                current.project_id,
                carried_capability.binding.resource().kind,
                carried_capability.binding.resource().id,
            )
            .await?
            {
                return Err(ReleaseServiceError::CapabilityResourceUnavailable);
            }
            authorize_capability_selection(self, &mut tx, identity, &carried_capability.binding)
                .await?;
        }
        let requirements = load_capability_requirements(&mut tx, current.release_agent_id).await?;
        diagnostics.extend(requirements.values().filter_map(|requirement| {
            let missing = requirement.slot_required()
                && !carried_capabilities
                    .iter()
                    .any(|carried| carried.binding.slot() == requirement.slot());
            missing.then(|| {
                json!({
                    "code": "required_capability_binding_missing",
                    "field": format!("capability_slots.{}", requirement.slot()),
                })
            })
        }));
        let runnable = diagnostics.is_empty();
        let cloned_capabilities = carried_capabilities
            .iter()
            .map(|carried| {
                CapabilityBinding::bind(
                    CapabilityBindingId::new(),
                    &carried.requirement,
                    carried.binding.resource(),
                    carried.binding.granted_operations(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let publication_repository_binding_id = current
            .publication_repository_slot
            .as_deref()
            .and_then(|slot| {
                cloned_capabilities
                    .iter()
                    .find(|binding| binding.slot().as_str() == slot)
            })
            .map(|binding| binding.id().as_uuid());
        insert_revision_with_binding_ids(
            &mut tx,
            &command,
            current.release_agent_id,
            &parameters,
            &effective,
            &current.publication_mode,
            publication_repository_binding_id,
            runnable,
            &diagnostics,
            &new_binding_ids,
            identity,
        )
        .await?;
        for (binding, binding_id) in carried.iter().zip(&new_binding_ids) {
            clone_revision_binding(
                &mut tx,
                *binding_id,
                command.new_revision_id,
                binding,
                identity,
            )
            .await?;
        }
        for (carried_capability, cloned) in carried_capabilities.iter().zip(&cloned_capabilities) {
            insert_capability_binding(
                &mut tx,
                command.new_revision_id,
                current.release_agent_id,
                cloned,
                &carried_capability.authorization_model_version,
                identity,
            )
            .await?;
            if let Some(git_binding) = &carried_capability.git_binding {
                insert_git_capability_binding(
                    &mut tx,
                    command.new_revision_id,
                    cloned,
                    git_binding,
                )
                .await?;
            }
        }
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state IN ('active', 'update_rejected')",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        record_command(
            &mut tx,
            command.command_key,
            "revise_instance",
            command.instance_id.as_uuid(),
            Some(command.new_revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.new_revision_id),
            "instance.revised",
            identity,
            json!({"runnable": runnable}),
        )
        .await?;
        append_event(
            &mut tx,
            command.instance_id.as_uuid(),
            "hephaestus.agent_instance.revised.v1",
            "agent_instance.revised.v1",
            json!({
                "schema_version": 1,
                "instance_id": command.instance_id,
                "revision_id": command.new_revision_id,
                "expected_revision_id": command.expected_revision_id,
                "runnable": runnable,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.new_revision_id)
    }

    /// Creates a release-update candidate. Invalid state capability, missing
    /// hook, parameter, secret, or policy candidates are persisted as rejected
    /// diagnostics without closing the run gate.
    ///
    /// # Errors
    ///
    /// Fails for denial, stale/concurrent update, family mismatch,
    /// idempotency conflict, invalid stored data, or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            instance_id = %command.instance_id,
            candidate_revision_id = %command.candidate_revision_id
        )
    )]
    pub async fn create_update(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateInstanceUpdate,
    ) -> Result<AgentUpdateId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(
                ObjectType::ReleaseAgent,
                command.candidate_release_agent_id.as_uuid(),
            ),
        )
        .await?;
        if let Some((id, _)) =
            existing_command(&mut tx, command.command_key, "create_update").await?
        {
            tx.commit().await?;
            return Ok(AgentUpdateId::from_uuid(id));
        }
        let current: UpdateCurrentRow = sqlx::query_as(
            "SELECT instance.active_revision_id, instance.family_id,
                    instance.project_id, instance.state, instance.run_gate_open,
                    current_agent.requires_state,
                    revision.secret_bindings
             FROM agent_instances AS instance
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS current_agent
               ON current_agent.id = revision.release_agent_id
             WHERE instance.id = $1
             FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if current.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        if !["active", "update_rejected"].contains(&current.state.as_str())
            || !current.run_gate_open
        {
            return Err(ReleaseServiceError::ConcurrentUpdate);
        }
        let candidate: UpdateCandidateRow = sqlx::query_as(
            "SELECT candidate.family_id, candidate.parameter_schema,
                    candidate.secret_slot_schema, candidate.runtime_contract,
                    candidate.requires_state, candidate.update_hook,
                    candidate.publication_mode,
                    candidate.publication_repository_slot
             FROM release_agents AS candidate
             JOIN releases AS release ON release.id = candidate.release_id
             WHERE candidate.id = $1 AND release.state = 'published'",
        )
        .bind(command.candidate_release_agent_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if candidate.family_id != current.family_id {
            return Err(ReleaseServiceError::AgentFamilyMismatch);
        }
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(candidate.parameter_schema)?;
        let release_policy = policy_from_contract(&candidate.runtime_contract)?;
        let effective = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let carried: Vec<RevisionBindingRow> = sqlx::query_as(
            "SELECT binding.id, binding.import_id, binding.slot_key,
                    binding.delivery_mode, binding.phases,
                    binding.attachment_ids, binding.destinations,
                    binding.effective_policy, binding.effective_policy_hash
             FROM agent_secret_bindings AS binding
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE binding.instance_revision_id = $1
               AND binding.status = 'active'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND secret.status = 'active'
               AND (source_grant.expires_at IS NULL
                    OR source_grant.expires_at > now())
             ORDER BY binding.slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let brokered_rules: Vec<BrokeredRuleCloneRow> = sqlx::query_as(
            "SELECT rule.id, rule.binding_id, secret.active_version_id AS secret_version_id,
                    rule.destination_origin, rule.location_kind,
                    rule.header_name, rule.header_prefix
             FROM brokered_secret_rules AS rule
             JOIN agent_secret_bindings AS binding
               ON binding.id = rule.binding_id
              AND binding.instance_revision_id = rule.instance_revision_id
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             WHERE rule.instance_revision_id = $1
               AND binding.status = 'active'
               AND binding.delivery_mode = 'brokered'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND (source_grant.expires_at IS NULL
                    OR source_grant.expires_at > now())
               AND secret.status = 'active'
               AND secret.active_version_id IS NOT NULL",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let expected: Vec<Uuid> = serde_json::from_value(current.secret_bindings)?;
        let mut diagnostics = Vec::new();
        let source_rule_ids = brokered_rules
            .iter()
            .map(|rule| rule.id)
            .collect::<BTreeSet<_>>();
        let (rule_copies, copy_diagnostics) =
            validate_brokered_rule_copies(&source_rule_ids, &command.brokered_rule_copies);
        diagnostics.extend(copy_diagnostics);
        for candidate_rule_id in rule_copies.values() {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                     SELECT 1 FROM brokered_secret_rules WHERE id = $1
                 )",
            )
            .bind(candidate_rule_id)
            .fetch_one(&mut *tx)
            .await?;
            if exists {
                diagnostics.push(json!({
                    "code": "brokered_rule_copy_target_conflict",
                    "field": "brokered_rule_copies"
                }));
            }
        }
        if carried.len() != expected.len() {
            diagnostics.push(json!({
                "code": "secret_binding_unavailable",
                "field": "secret_slots"
            }));
        }
        for binding in &carried {
            self.require(
                &mut tx,
                identity,
                if binding.delivery_mode == "raw" {
                    Permission::BindRaw
                } else {
                    Permission::BindBrokered
                },
                ObjectRef::new(ObjectType::SecretImport, binding.import_id),
            )
            .await?;
            if !candidate_accepts_binding(&candidate.secret_slot_schema, binding) {
                diagnostics.push(json!({
                    "code": "secret_binding_incompatible",
                    "field": format!("secret_slots.{}", binding.slot_key)
                }));
            }
        }
        let bound_slots = carried
            .iter()
            .map(|binding| binding.slot_key.as_str())
            .collect::<std::collections::HashSet<_>>();
        diagnostics.extend(required_slot_diagnostics(
            &candidate.secret_slot_schema,
            &bound_slots,
        )?);
        diagnostics.extend(update_contract_diagnostics(
            current.requires_state,
            candidate.requires_state,
            candidate.update_hook.as_ref(),
        ));
        let carried_capabilities =
            load_carried_capability_bindings(&mut tx, command.expected_revision_id).await?;
        let candidate_requirements =
            load_capability_requirements(&mut tx, command.candidate_release_agent_id.as_uuid())
                .await?;
        let candidate_git_ceilings =
            load_git_ceilings(&mut tx, command.candidate_release_agent_id.as_uuid()).await?;
        let mut candidate_capabilities = Vec::new();
        for carried in &carried_capabilities {
            let Some(requirement) = candidate_requirements.get(carried.binding.slot()) else {
                continue;
            };
            let Ok(binding) = CapabilityBinding::bind(
                CapabilityBindingId::new(),
                requirement,
                carried.binding.resource(),
                carried.binding.granted_operations(),
            ) else {
                diagnostics.push(json!({
                    "code": "capability_binding_incompatible",
                    "field": format!("capability_slots.{}", carried.binding.slot()),
                }));
                continue;
            };
            if !capability_resource_is_in_project(
                &mut tx,
                current.project_id,
                binding.resource().kind,
                binding.resource().id,
            )
            .await?
            {
                diagnostics.push(json!({
                    "code": "capability_resource_unavailable",
                    "field": format!("capability_slots.{}", binding.slot()),
                }));
                continue;
            }
            authorize_capability_selection(self, &mut tx, identity, &binding).await?;
            let git_binding = match (
                &carried.git_binding,
                candidate_git_ceilings.get(&requirement.id()),
            ) {
                (Some(source), Some(ceiling)) => Some(BoundGitCapability::new(
                    GitRepositoryId::new(binding.resource().id),
                    source.authority().clone(),
                    ceiling,
                )?),
                (None, None) => None,
                (Some(_), None) | (None, Some(_)) => {
                    diagnostics.push(json!({
                        "code": "capability_binding_incompatible",
                        "field": format!("capability_slots.{}", binding.slot()),
                    }));
                    continue;
                }
            };
            candidate_capabilities.push((
                binding,
                carried.authorization_model_version.clone(),
                git_binding,
            ));
        }
        diagnostics.extend(candidate_requirements.values().filter_map(|requirement| {
            let missing = requirement.slot_required()
                && !candidate_capabilities
                    .iter()
                    .any(|(binding, _, _)| binding.slot() == requirement.slot());
            missing.then(|| {
                json!({
                    "code": "required_capability_binding_missing",
                    "field": format!("capability_slots.{}", requirement.slot()),
                })
            })
        }));
        let runnable = diagnostics.is_empty();
        let publication_repository_binding_id = candidate
            .publication_repository_slot
            .as_deref()
            .and_then(|slot| {
                candidate_capabilities
                    .iter()
                    .find(|(binding, _, _)| binding.slot().as_str() == slot)
            })
            .map(|(binding, _, _)| binding.id().as_uuid());
        let new_binding_ids = carried.iter().map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        insert_update_candidate_revision(
            &mut tx,
            &command,
            &parameters,
            &effective,
            &candidate.publication_mode,
            publication_repository_binding_id,
            runnable,
            &diagnostics,
            &new_binding_ids,
            identity,
        )
        .await?;
        for (binding, binding_id) in carried.iter().zip(&new_binding_ids) {
            clone_revision_binding(
                &mut tx,
                *binding_id,
                command.candidate_revision_id,
                binding,
                identity,
            )
            .await?;
        }
        for rule in &brokered_rules {
            let Some(candidate_rule_id) = rule_copies.get(&rule.id).copied() else {
                continue;
            };
            let binding_id = carried
                .iter()
                .zip(&new_binding_ids)
                .find_map(|(binding, binding_id)| {
                    (binding.id == rule.binding_id).then_some(*binding_id)
                })
                .ok_or(ReleaseServiceError::InvalidStoredData)?;
            clone_brokered_secret_rule(
                &mut tx,
                rule,
                binding_id,
                command.candidate_revision_id,
                candidate_rule_id,
            )
            .await?;
        }
        for (binding, authorization_model_version, git_binding) in &candidate_capabilities {
            insert_capability_binding(
                &mut tx,
                command.candidate_revision_id,
                command.candidate_release_agent_id.as_uuid(),
                binding,
                authorization_model_version,
                identity,
            )
            .await?;
            if let Some(git_binding) = git_binding {
                insert_git_capability_binding(
                    &mut tx,
                    command.candidate_revision_id,
                    binding,
                    git_binding,
                )
                .await?;
            }
        }
        let update_state = if runnable { "draining" } else { "rejected" };
        sqlx::query(
            "INSERT INTO agent_updates
             (id, instance_id, expected_current_revision_id,
              candidate_revision_id, state, diagnostics, final_decision,
              actor_id, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6,
                     CASE WHEN $5 = 'rejected' THEN 'agent_rejected' END,
                     $7, CASE WHEN $5 = 'rejected' THEN now() END)",
        )
        .bind(command.update_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.candidate_revision_id.as_uuid())
        .bind(update_state)
        .bind(serde_json::to_value(&diagnostics)?)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if runnable {
            let closed = sqlx::query(
                "UPDATE agent_instances
                 SET run_gate_open = false, state = 'update_draining',
                     version = version + 1, updated_at = now()
                 WHERE id = $1 AND active_revision_id = $2
                   AND run_gate_open AND state IN ('active', 'update_rejected')",
            )
            .bind(command.instance_id.as_uuid())
            .bind(command.expected_revision_id.as_uuid())
            .execute(&mut *tx)
            .await?;
            if closed.rows_affected() != 1 {
                return Err(ReleaseServiceError::ConcurrentUpdate);
            }
        }
        record_command(
            &mut tx,
            command.command_key,
            "create_update",
            command.update_id.as_uuid(),
            Some(command.candidate_revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.candidate_revision_id),
            if runnable {
                "update.draining"
            } else {
                "update.rejected"
            },
            identity,
            json!({
                "update_id": command.update_id,
                "diagnostics": diagnostics,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.requested.v1",
            "agent_update.requested.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "instance_id": command.instance_id,
                "expected_revision_id": command.expected_revision_id,
                "candidate_revision_id": command.candidate_revision_id,
                "state": update_state,
            }),
        )
        .await?;
        if !runnable {
            append_event(
                &mut tx,
                command.update_id.as_uuid(),
                "hephaestus.agent_update.rejected.v1",
                "agent_update.rejected.v1",
                json!({
                    "update_id": command.update_id,
                    "instance_id": command.instance_id,
                    "expected_revision_id": command.expected_revision_id,
                    "candidate_revision_id": command.candidate_revision_id,
                    "reason": "candidate_not_runnable",
                    "diagnostics": diagnostics,
                }),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(command.update_id)
    }

    /// Enters the isolated hook only after all pre-gate normal work drains and
    /// creates the exact queued update run. The run orchestrator acquires the
    /// optional state volume under its normal fenced exclusive lease.
    ///
    /// # Errors
    ///
    /// Fails for denial, undrained work, stale lifecycle, idempotency conflict,
    /// or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            run_id = %command.hook_run_id
        )
    )]
    pub async fn begin_update_hook(
        &self,
        identity: &AuthenticatedIdentity,
        command: BeginUpdateHook,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let update: UpdateHookAdmissionRow = sqlx::query_as(
            "SELECT update.instance_id, update.candidate_revision_id, update.state,
                    instance.state AS instance_state,
                    update.created_at,
                    candidate.release_agent_id,
                    release_agent.release_id,
                    release_agent.requires_state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             JOIN agent_instance_revisions AS candidate
               ON candidate.id = update.candidate_revision_id
              AND candidate.instance_id = update.instance_id
             JOIN release_agents AS release_agent
               ON release_agent.id = candidate.release_agent_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(command.update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, update.instance_id),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "begin_update_hook")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        if update.state != "draining" || update.instance_state != "update_draining" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_requests
                 WHERE instance_id = $1
                   AND request_kind = 'instance_normal'
                   AND dispatch_state = 'pending'
                   AND created_at <= $2
             ) OR EXISTS(
                 SELECT 1
                 FROM run_instance_provenance AS provenance
                 JOIN runs ON runs.id = provenance.run_id
                 WHERE provenance.instance_id = $1
                   AND provenance.phase = 'normal'
                   AND runs.state <> 'cleaned_up'
             ) OR EXISTS(
                 SELECT 1
                 FROM runs
                 WHERE runs.instance_id = $1
                   AND runs.run_kind = 'normal'
                   AND runs.state <> 'cleaned_up'
             )",
        )
        .bind(update.instance_id)
        .bind(update.created_at)
        .fetch_one(&mut *tx)
        .await?;
        if pending {
            return Err(ReleaseServiceError::UpdateDrainPending);
        }
        let start_id = CommandId::new();
        let insert_result = sqlx::query(
            "INSERT INTO runs
             (id, instance_id, instance_revision_id, release_id,
              release_agent_id, run_kind, command_id, state, requires_state,
              created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', $7,
                     now(), now())",
        )
        .bind(command.hook_run_id.as_uuid())
        .bind(update.instance_id)
        .bind(update.candidate_revision_id)
        .bind(update.release_id)
        .bind(update.release_agent_id)
        .bind(start_id.as_uuid())
        .bind(update.requires_state)
        .execute(&mut *tx)
        .await;
        match insert_result {
            Ok(_) => {}
            Err(error) if is_update_admission_generation_conflict(&error) => {
                return Err(ReleaseServiceError::UpdateAdmissionGenerationRace);
            }
            Err(error) => return Err(error.into()),
        }
        let changed = sqlx::query(
            "UPDATE agent_updates
             SET state = 'hook_running', hook_run_id = $2, updated_at = now()
             WHERE id = $1 AND state = 'draining'",
        )
        .bind(command.update_id.as_uuid())
        .bind(command.hook_run_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        sqlx::query(
            "UPDATE agent_instances
             SET state = 'updating', version = version + 1, updated_at = now()
             WHERE id = $1 AND state = 'update_draining'
               AND NOT run_gate_open",
        )
        .bind(update.instance_id)
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "begin_update_hook",
            command.update_id.as_uuid(),
            Some(command.hook_run_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.hook_started.v1",
            "agent_update.hook_started.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "hook_run_id": command.hook_run_id,
            }),
        )
        .await?;
        let start = StartRun {
            command_id: start_id,
            run_id: command.hook_run_id,
            instance_id: AgentInstanceId::from_uuid(update.instance_id),
            instance_revision_id: AgentInstanceRevisionId::from_uuid(update.candidate_revision_id),
            release_id: ReleaseId::from_uuid(update.release_id),
            release_agent_id: ReleaseAgentId::from_uuid(update.release_agent_id),
            attachment_id: None,
            kind: RunKind::Update,
            requires_state: update.requires_state,
        };
        append_event(
            &mut tx,
            command.hook_run_id.as_uuid(),
            RUN_START_SUBJECT,
            "run.start.v1",
            serde_json::to_value(start)?,
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Reads the hook run durably admitted for an update after a concurrent
    /// reconciler wins the admission race.
    ///
    /// # Errors
    ///
    /// Fails when the update is unavailable, the actor is unauthorized, or
    /// `PostgreSQL` cannot read the lifecycle row.
    pub async fn current_update_hook_run(
        &self,
        identity: &AuthenticatedIdentity,
        update_id: AgentUpdateId,
    ) -> Result<Option<RunId>, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let instance_id: Uuid =
            sqlx::query_scalar("SELECT instance_id FROM agent_updates WHERE id = $1 FOR UPDATE")
                .bind(update_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, instance_id),
        )
        .await?;
        let hook_run_id: Option<Uuid> =
            sqlx::query_scalar("SELECT hook_run_id FROM agent_updates WHERE id = $1")
                .bind(update_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(hook_run_id.map(RunId::from_uuid))
    }

    /// Applies an explicit operator decision to a paused update.
    ///
    /// Retrying is permitted only from the compatibility-unknown path and
    /// retains the stable update ID so an agent can deduplicate its own hook.
    /// Rejecting reopens the prior revision without claiming that Hephaestus
    /// rolled agent-owned state back. Resuming activation is permitted only
    /// after durable hook success.
    ///
    /// # Errors
    ///
    /// Fails for denial, an action/lifecycle mismatch, idempotency conflict,
    /// stale revision state, or database failure.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            update_id = %command.update_id,
            action = command.action.operation()
        )
    )]
    pub async fn recover_update(
        &self,
        identity: &AuthenticatedIdentity,
        command: RecoverInstanceUpdate,
    ) -> Result<UpdateRecoveryDecision, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let update: UpdateRecoveryRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state,
                    instance.active_revision_id,
                    instance.state AS instance_state,
                    instance.run_gate_open
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(command.update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRecover,
            ObjectRef::new(ObjectType::AgentInstance, update.instance_id),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, command.action.operation())
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(recovery_decision(command.action));
        }

        let event_type = match command.action {
            UpdateRecoveryAction::RetryHook => {
                if update.state != "compatibility_unknown"
                    || update.instance_state != "paused_unknown_state"
                    || update.run_gate_open
                    || update.active_revision_id != Some(update.expected_current_revision_id)
                {
                    return Err(ReleaseServiceError::InvalidUpdateLifecycle);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'draining', hook_run_id = NULL,
                         hook_exit_code = NULL, hook_exit_signal = NULL,
                         final_decision = NULL, completed_at = NULL,
                         actor_id = $3,
                         diagnostics = diagnostics || $2::jsonb,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(command.update_id.as_uuid())
                .bind(json!([{
                    "code": "operator_retry_after_uncertain_hook",
                    "field": "update_hook"
                }]))
                .bind(identity.user_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_instances
                     SET state = 'update_draining', version = version + 1,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, command.update_id, "released").await?;
                "update.recovery_retry_scheduled"
            }
            UpdateRecoveryAction::RejectCandidate => {
                if update.state != "compatibility_unknown"
                    || update.instance_state != "paused_unknown_state"
                    || update.run_gate_open
                    || update.active_revision_id != Some(update.expected_current_revision_id)
                {
                    return Err(ReleaseServiceError::InvalidUpdateLifecycle);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'rejected', final_decision = 'recovery',
                         diagnostics = diagnostics || $2::jsonb,
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(command.update_id.as_uuid())
                .bind(json!([{
                    "code": "operator_rejected_uncertain_candidate",
                    "field": "agent_owned_state"
                }]))
                .execute(&mut *tx)
                .await?;
                reopen_after_update(
                    &mut tx,
                    command.update_id,
                    update.instance_id,
                    update.expected_current_revision_id,
                    "update_rejected",
                )
                .await?;
                "update.recovery_candidate_rejected"
            }
            UpdateRecoveryAction::ResumeActivation => {
                let active_revision = update
                    .active_revision_id
                    .ok_or(ReleaseServiceError::InvalidUpdateLifecycle)?;
                if update.state != "activation_recovery"
                    || update.instance_state != "paused_activation_recovery"
                    || update.run_gate_open
                    || ![
                        update.expected_current_revision_id,
                        update.candidate_revision_id,
                    ]
                    .contains(&active_revision)
                {
                    return Err(ReleaseServiceError::InvalidUpdateLifecycle);
                }
                sqlx::query(
                    "UPDATE agent_instances
                     SET active_revision_id = $2, state = 'active',
                         run_gate_open = true, version = version + 1,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .bind(update.candidate_revision_id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'activated', final_decision = 'activated',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(command.update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, command.update_id, "released").await?;
                materialize_deferred_triggers(
                    &mut tx,
                    update.instance_id,
                    update.candidate_revision_id,
                )
                .await?;
                enqueue_mailbox_wakes(&mut tx, update.instance_id).await?;
                "update.recovery_activation_resumed"
            }
        };
        record_command(
            &mut tx,
            command.command_key,
            command.action.operation(),
            command.update_id.as_uuid(),
            Some(update.candidate_revision_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(update.instance_id),
            Some(AgentInstanceRevisionId::from_uuid(
                update.candidate_revision_id,
            )),
            event_type,
            identity,
            json!({
                "update_id": command.update_id,
                "action": command.action.operation(),
                "host_rollback_claimed": false,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.update_id.as_uuid(),
            "hephaestus.agent_update.recovered.v1",
            "agent_update.recovered.v1",
            json!({
                "schema_version": 1,
                "update_id": command.update_id,
                "instance_id": update.instance_id,
                "action": command.action.operation(),
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(recovery_decision(command.action))
    }

    /// Records the hook terminal contract. Exit zero is the irreversible
    /// commit point but activation is a separate reconciliable transaction.
    ///
    /// # Errors
    ///
    /// Fails for stale lifecycle or database errors.
    #[tracing::instrument(skip_all, fields(%update_id))]
    pub async fn record_update_hook_result(
        &self,
        update_id: AgentUpdateId,
        result: UpdateHookResult,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let mut tx = self.pool.begin().await?;
        let update: UpdateLifecycleRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if update.state != "hook_running" {
            return existing_terminal_decision(&update.state);
        }
        let decision = match result {
            UpdateHookResult::Committed => {
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'hook_committed', hook_exit_code = 0,
                         updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                append_event(
                    &mut tx,
                    update_id.as_uuid(),
                    "hephaestus.agent_update.hook_committed.v1",
                    "agent_update.hook_committed.v1",
                    json!({"schema_version": 1, "update_id": update_id}),
                )
                .await?;
                UpdateDecision::ActivationRecovery
            }
            UpdateHookResult::Rejected(exit_code) => {
                if exit_code == 0 {
                    return Err(ReleaseServiceError::InvalidHookResult);
                }
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'rejected', hook_exit_code = $2,
                         final_decision = 'agent_rejected',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .bind(exit_code)
                .execute(&mut *tx)
                .await?;
                reopen_after_update(
                    &mut tx,
                    update_id,
                    update.instance_id,
                    update.expected_current_revision_id,
                    "update_rejected",
                )
                .await?;
                append_rejected_update_event(&mut tx, update_id, &update, exit_code).await?;
                UpdateDecision::AgentRejected
            }
            UpdateHookResult::Uncertain => {
                sqlx::query(
                    "UPDATE agent_updates
                     SET state = 'compatibility_unknown',
                         final_decision = 'unknown',
                         completed_at = now(), updated_at = now()
                     WHERE id = $1",
                )
                .bind(update_id.as_uuid())
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "UPDATE agent_instances
                     SET state = 'paused_unknown_state', run_gate_open = false,
                         version = version + 1, updated_at = now()
                     WHERE id = $1",
                )
                .bind(update.instance_id)
                .execute(&mut *tx)
                .await?;
                mark_volume_lease(&mut tx, update_id, "recovery_required").await?;
                append_uncertain_update_events(
                    &mut tx,
                    update_id,
                    &update,
                    "hook_result_uncertain",
                    "paused_unknown_state",
                )
                .await?;
                UpdateDecision::CompatibilityUnknown
            }
        };
        tx.commit().await?;
        Ok(decision)
    }

    /// Reconciles one cleaned update run into the durable update state machine.
    ///
    /// Successful hooks are activated immediately; explicit nonzero exits
    /// reopen the prior revision; every ambiguous terminal path pauses the
    /// instance for recovery. Repeated calls are idempotent.
    ///
    /// # Errors
    ///
    /// Returns a stable lifecycle error until the exact hook run is cleaned,
    /// or a database error when its result cannot be persisted.
    #[tracing::instrument(skip_all, fields(%run_id))]
    pub async fn reconcile_update_run(
        &self,
        run_id: RunId,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let row: UpdateRunResultRow = sqlx::query_as(
            "SELECT update.id AS update_id, update.state AS update_state,
                    run.state AS run_state, run.outcome, run.exit_code,
                    run.exit_signal, run.failure
             FROM agent_updates AS update
             JOIN runs AS run ON run.id = update.hook_run_id
             WHERE update.hook_run_id = $1 AND run.run_kind = 'update'",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        let update_id = AgentUpdateId::from_uuid(row.update_id);
        match row.update_state.as_str() {
            "hook_committed" => return self.activate_committed_update(update_id).await,
            "hook_running" => {}
            state => return existing_terminal_decision(state),
        }
        if row.run_state != "cleaned_up" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let result = match (
            row.outcome.as_deref(),
            row.exit_code,
            row.exit_signal,
            row.failure.as_deref(),
        ) {
            (Some("succeeded"), Some(0), None, _) => UpdateHookResult::Committed,
            (Some("failed"), Some(code), None, None) if code != 0 => {
                UpdateHookResult::Rejected(code)
            }
            (Some("failed" | "cancelled"), _, _, _) => UpdateHookResult::Uncertain,
            _ => return Err(ReleaseServiceError::InvalidHookResult),
        };
        let decision = self.record_update_hook_result(update_id, result).await?;
        if result == UpdateHookResult::Committed {
            self.activate_committed_update(update_id).await
        } else {
            Ok(decision)
        }
    }

    /// Activates an exact hook-committed candidate without re-running the hook
    /// or allowing later source revocation to veto the committed migration.
    ///
    /// # Errors
    ///
    /// Returns database failures; CAS anomalies are durably converted to
    /// activation-recovery state and returned as a decision.
    #[tracing::instrument(skip_all, fields(%update_id))]
    pub async fn activate_committed_update(
        &self,
        update_id: AgentUpdateId,
    ) -> Result<UpdateDecision, ReleaseServiceError> {
        let mut tx = self.pool.begin().await?;
        let update: UpdateLifecycleRow = sqlx::query_as(
            "SELECT update.instance_id, update.expected_current_revision_id,
                    update.candidate_revision_id, update.state
             FROM agent_updates AS update
             JOIN agent_instances AS instance ON instance.id = update.instance_id
             WHERE update.id = $1
             FOR UPDATE OF update, instance",
        )
        .bind(update_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        if update.state == "activated" {
            tx.commit().await?;
            return Ok(UpdateDecision::Activated);
        }
        if update.state != "hook_committed" {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, state = 'active',
                 run_gate_open = true, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state = 'updating' AND NOT run_gate_open",
        )
        .bind(update.instance_id)
        .bind(update.expected_current_revision_id)
        .bind(update.candidate_revision_id)
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            sqlx::query(
                "UPDATE agent_updates
                 SET state = 'activation_recovery',
                     final_decision = 'recovery', updated_at = now()
                 WHERE id = $1",
            )
            .bind(update_id.as_uuid())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE agent_instances
                 SET state = 'paused_activation_recovery',
                     run_gate_open = false, version = version + 1,
                     updated_at = now()
                 WHERE id = $1",
            )
            .bind(update.instance_id)
            .execute(&mut *tx)
            .await?;
            mark_volume_lease(&mut tx, update_id, "recovery_required").await?;
            append_uncertain_update_events(
                &mut tx,
                update_id,
                &update,
                "activation_compare_and_swap_failed",
                "paused_activation_recovery",
            )
            .await?;
            tx.commit().await?;
            return Ok(UpdateDecision::ActivationRecovery);
        }
        sqlx::query(
            "UPDATE agent_updates
             SET state = 'activated', final_decision = 'activated',
                 completed_at = now(), updated_at = now()
             WHERE id = $1",
        )
        .bind(update_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        mark_volume_lease(&mut tx, update_id, "released").await?;
        materialize_deferred_triggers(&mut tx, update.instance_id, update.candidate_revision_id)
            .await?;
        enqueue_mailbox_wakes(&mut tx, update.instance_id).await?;
        append_event(
            &mut tx,
            update_id.as_uuid(),
            "hephaestus.agent_update.completed.v1",
            "agent_update.completed.v1",
            json!({
                "update_id": update_id,
                "instance_id": update.instance_id,
                "revision_id": update.candidate_revision_id,
                "decision": "activated",
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(UpdateDecision::Activated)
    }

    async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), ReleaseServiceError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // The command transaction will roll back on denial. Persist the
            // denial independently so rejected privileged attempts remain
            // observable without committing any command-side state.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                permission,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(ReleaseServiceError::AuthorizationDenied)
        }
    }
}

/// Stable non-sensitive release service failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReleaseServiceError {
    /// Exact authorization was denied.
    #[error("release or instance command is not authorized")]
    AuthorizationDenied,
    /// Referenced build/release/agent/instance is unavailable.
    #[error("release or instance authority is unavailable")]
    Unavailable,
    /// Build is not at its sealed import boundary.
    #[error("build is not ready for immutable artifact import")]
    BuildNotImporting,
    /// No valid reusable configuration exists at the exact build commit.
    #[error("reusable release configuration is missing")]
    ReusableConfigurationMissing,
    /// Artifact set is empty.
    #[error("release artifact manifest is incomplete")]
    IncompleteArtifacts,
    /// Artifact metadata violates bounds.
    #[error("release artifact metadata is invalid")]
    InvalidArtifact,
    /// Typed parameter diagnostics prevented a revision.
    #[error("agent instance parameters are invalid")]
    InvalidParameters(Vec<release_domain::ParameterDiagnostic>),
    /// Stored JSON/provenance violates a domain invariant.
    #[error("stored release data is invalid")]
    InvalidStoredData,
    /// Expected active revision lost its compare-and-swap race.
    #[error("agent instance revision changed concurrently")]
    StaleInstanceRevision,
    /// One carried secret binding is no longer live and bindable.
    #[error("agent instance secret binding is unavailable")]
    SecretBindingUnavailable,
    /// Capability declaration or binding violates the immutable release ceiling.
    #[error(transparent)]
    Capability(#[from] CapabilityError),
    /// Typed Git ceiling or attenuation is malformed.
    #[error("runtime Git authority is invalid")]
    GitCapability(#[from] git_capability_domain::GitCapabilityError),
    /// Capability selection is duplicated, unknown, or otherwise malformed.
    #[error("agent instance capability binding is invalid")]
    InvalidCapabilityBinding,
    /// Exact resource is missing, outside the project, or not implemented.
    #[error("agent instance capability resource is unavailable")]
    CapabilityResourceUnavailable,
    /// Another update already closed the instance run gate.
    #[error("agent instance already has an active update")]
    ConcurrentUpdate,
    /// Candidate export belongs to another source agent family.
    #[error("agent update candidate belongs to another family")]
    AgentFamilyMismatch,
    /// Update or instance is not at the requested durable boundary.
    #[error("agent update lifecycle does not permit this operation")]
    InvalidUpdateLifecycle,
    /// Pre-gate normal requests or runs have not drained.
    #[error("agent update is waiting for normal runs to drain")]
    UpdateDrainPending,
    /// A concurrent update-hook admission won the durable run identity race.
    #[error("agent update hook admission raced another generation")]
    UpdateAdmissionGenerationRace,
    /// Persistent state volume is not ready for an exclusive update lease.
    #[error("agent update state volume is unavailable")]
    UpdateVolumeUnavailable,
    /// Update lease host or expiry violates bounds.
    #[error("agent update volume lease is invalid")]
    InvalidUpdateLease,
    /// Hook exit result is malformed.
    #[error("agent update hook result is invalid")]
    InvalidHookResult,
    /// One command identity was reused for another operation.
    #[error("release command idempotency identity conflicts")]
    IdempotencyConflict,
    /// Release domain validation failure.
    #[error(transparent)]
    Domain(#[from] release_domain::ReleaseValueError),
    /// Authorization provider failure.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// JSON serialization failure.
    #[error("release configuration serialization failed")]
    Serialization(#[from] serde_json::Error),
    /// Database failure.
    #[error("release persistence failed")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod release_helper_tests;

#[cfg(test)]
mod ui_schema_tests;
