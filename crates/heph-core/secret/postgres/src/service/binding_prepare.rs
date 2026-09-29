use super::*;

pub(super) struct BindingPreparation {
    pub(super) revision: RevisionCloneRow,
    pub(super) import: EligibleImportRow,
    pub(super) copied: Vec<(AgentSecretBindingId, CarriedBindingRow)>,
    pub(super) binding_ids: Vec<Uuid>,
    pub(super) diagnostics: serde_json::Value,
    pub(super) runnable: bool,
    pub(super) carried_capabilities: Vec<CarriedCapabilityRow>,
    pub(super) cloned_capabilities: Vec<CapabilityBinding>,
    pub(super) publication_repository_binding_id: Option<Uuid>,
}

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    #[allow(clippy::too_many_lines)] // Keep validation and authority reads in one transaction phase.
    pub(super) async fn prepare_binding_revision(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        command: &BindSecret,
    ) -> Result<BindingPreparation, SecretServiceError> {
        let revision: RevisionCloneRow = sqlx::query_as(
            "SELECT instance.project_id, instance.active_revision_id,
                      revision.release_agent_id, revision.parameters,
                      revision.parameter_hash, revision.resource_selection,
                      revision.network_restriction,
                      revision.effective_runtime_policy,
                      revision.effective_policy_hash,
                      revision.platform_policy_version,
                      revision.publication_repository_binding_id,
                      agent.secret_slot_schema
               FROM agent_instances AS instance
               JOIN agent_instance_revisions AS revision
                 ON revision.id = instance.active_revision_id
               JOIN release_agents AS agent ON agent.id = revision.release_agent_id
               WHERE instance.id = $1
                 AND instance.state IN ('active', 'update_rejected')
               FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        if revision.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(SecretServiceError::StaleInstanceRevision);
        }
        let declaration = declared_slot(&revision.secret_slot_schema, command.slot.as_str())?;
        validate_declared_binding(&declaration, command)?;
        let import = load_eligible_import(tx, command.import_id)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        validate_import_policy(&import, command)?;
        validate_binding_scope(
            tx,
            command.instance_id,
            revision.project_id,
            &import,
            command,
        )
        .await?;

        let carried: Vec<CarriedBindingRow> = sqlx::query_as(
            "SELECT import_id, slot_key, delivery_mode, phases,
                      attachment_ids, destinations, effective_policy,
                      effective_policy_hash
               FROM agent_secret_bindings
               WHERE instance_revision_id = $1 AND status = 'active'
                 AND slot_key <> $2
               ORDER BY slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.slot.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let mut copied = Vec::with_capacity(carried.len());
        for binding in carried {
            let mode = parse_mode(&binding.delivery_mode)?;
            let import_id = SecretImportId::from_uuid(binding.import_id);
            self.require_binding_mode(tx, identity, import_id, mode)
                .await?;
            let live = load_eligible_import(tx, import_id)
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            validate_carried_policy(&live, &binding)?;
            copied.push((AgentSecretBindingId::new(), binding));
        }
        let mut binding_ids = copied
            .iter()
            .map(|(id, _)| id.as_uuid())
            .collect::<Vec<_>>();
        binding_ids.push(command.binding_id.as_uuid());
        let diagnostics = unresolved_required_diagnostics(
            &revision.secret_slot_schema,
            copied
                .iter()
                .map(|(_, binding)| binding.slot_key.as_str())
                .chain(std::iter::once(command.slot.as_str())),
        )?;
        let runnable = diagnostics.as_array().is_some_and(std::vec::Vec::is_empty);
        let carried_capabilities: Vec<CarriedCapabilityRow> = sqlx::query_as(
            "SELECT binding.id AS source_binding_id, binding.requirement_id,
                    binding.slot_key, requirement.resource_kind AS requirement_resource_kind,
                    requirement.required_operations,
                    requirement.optional_operations, requirement.slot_required,
                    binding.resource_kind, binding.resource_id,
                    binding.granted_operations, binding.authorization_model_version
             FROM agent_capability_bindings AS binding
             JOIN release_capability_requirements AS requirement
               ON requirement.id = binding.requirement_id
              AND requirement.release_agent_id = binding.release_agent_id
             WHERE binding.instance_revision_id = $1
             ORDER BY binding.slot_key, binding.id",
        )
        .bind(command.expected_revision_id.as_uuid())
        .fetch_all(&mut **tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let cloned_capabilities = carried_capabilities
            .iter()
            .map(clone_capability_binding)
            .collect::<Result<Vec<_>, _>>()?;
        let publication_repository_binding_id = revision
            .publication_repository_binding_id
            .and_then(|source_id| {
                carried_capabilities
                    .iter()
                    .zip(&cloned_capabilities)
                    .find(|(binding, _)| binding.source_binding_id == source_id)
                    .map(|(_, cloned)| cloned.id().as_uuid())
            });
        Ok(BindingPreparation {
            revision,
            import,
            copied,
            binding_ids,
            diagnostics,
            runnable,
            carried_capabilities,
            cloned_capabilities,
            publication_repository_binding_id,
        })
    }
}
