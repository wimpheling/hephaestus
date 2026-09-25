//! Instance parameter and policy revision commands.

use super::{
    AgentInstanceRevisionId, AuthenticatedIdentity, CapabilityBinding, CapabilityBindingId,
    ObjectRef, ObjectType, ParameterDeclaration, ParameterDocument, Permission, ReleaseService,
    ReleaseServiceError, ReviseInstance, RevisionBindingRow, RevisionUpdateRow, RuntimePolicy,
    append_event, append_instance_event, authorize_capability_selection, begin_actor_transaction,
    capability_resource_is_in_project, clone_revision_binding, existing_command,
    insert_capability_binding, insert_git_capability_binding, insert_revision_with_binding_ids,
    load_capability_requirements, load_carried_capability_bindings, policy_from_contract,
    record_command, required_slot_diagnostics,
};
use serde_json::json;
use uuid::Uuid;

impl ReleaseService {
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
}
