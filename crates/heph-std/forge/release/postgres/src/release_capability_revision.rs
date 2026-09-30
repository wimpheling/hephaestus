use super::{
    AgentInstanceRevisionId, AuthenticatedIdentity, CapabilityBinding, CapabilityRequirementRow,
    CapabilityRevisionDiagnostic, CapabilityRevisionResult, CapabilityRevisionSourceRow, ObjectRef,
    ObjectType, Permission, ReleaseService, ReleaseServiceError, ReviseInstanceCapabilities,
    RevisionBindingRow, append_instance_event, authorize_capability_selection,
    begin_actor_transaction, capability_diagnostics_from_json, capability_resource_is_in_project,
    clone_revision_binding, existing_command, git_authority_matches_capability,
    insert_capability_binding, insert_git_capability_binding, load_git_ceilings, record_command,
    stored_requirement,
};
use git_capability_domain::{BoundGitCapability, RepositoryId as GitRepositoryId};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

impl ReleaseService {
    /// Creates and compare-and-swap activates an immutable revision with an explicit capability binding set.
    /// Existing fields are cloned and bindings revalidated; the previous revision remains unchanged.
    ///
    /// # Errors
    ///
    /// Fails for denial, stale or malformed bindings, unavailable resources, idempotency conflict,
    /// corrupt requirements, or database failure.
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
}
