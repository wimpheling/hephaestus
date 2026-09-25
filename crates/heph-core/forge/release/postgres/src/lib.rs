//! `PostgreSQL` release and instance command adapter.

use agent_config::{AgentConfig, NetworkProfile, ParameterDefault, REUSABLE_RELEASE_VERSION};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
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
pub use release_errors::ReleaseServiceError;
use release_revision_persistence::{
    clone_revision_binding, insert_revision, insert_revision_with_binding_ids,
    insert_update_candidate_revision,
};
use release_rows::{
    BrokeredRuleCloneRow, CapabilityRequirementRow, CapabilityRevisionSourceRow,
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
mod release_attachments;
mod release_brokered_helpers;
mod release_build;
mod release_capability_binding;
mod release_capability_helpers;
mod release_capability_revision;
mod release_capability_selection;
mod release_command_events;
mod release_errors;
mod release_instance_import;
mod release_instance_revision;
mod release_publication;
mod release_revision_persistence;
mod release_rows;
mod release_service_access;
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

#[cfg(test)]
mod release_helper_tests;

#[cfg(test)]
mod ui_schema_tests;
