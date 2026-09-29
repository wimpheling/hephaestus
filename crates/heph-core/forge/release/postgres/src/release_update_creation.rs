//! Release update creation command.

use super::release_update_preparation::{PreparedUpdate, prepare_update};
use super::{
    AgentUpdateId, AuthenticatedIdentity, CreateInstanceUpdate, ObjectRef, ObjectType, Permission,
    ReleaseService, ReleaseServiceError, UpdateCandidateRow, UpdateCurrentRow, append_event,
    append_instance_event, begin_actor_transaction, clone_brokered_secret_rule,
    clone_revision_binding, existing_command, insert_capability_binding,
    insert_git_capability_binding, insert_update_candidate_revision, record_command,
};
use serde_json::json;

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
        let PreparedUpdate {
            publication_mode,
            parameters,
            effective,
            carried,
            brokered_rules,
            rule_copies,
            candidate_capabilities,
            diagnostics,
            runnable,
            publication_repository_binding_id,
            new_binding_ids,
        } = prepare_update(&mut tx, self, identity, &command, current, candidate).await?;

        insert_update_candidate_revision(
            &mut tx,
            &command,
            &parameters,
            &effective,
            &publication_mode,
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
}
