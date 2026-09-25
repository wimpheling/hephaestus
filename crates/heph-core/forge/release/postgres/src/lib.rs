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
    update_contract_diagnostics,
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
mod release_update_admission;
mod release_update_creation;
mod release_update_gate;
mod release_update_preparation;
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
