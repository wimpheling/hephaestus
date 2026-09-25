use super::{
    AgentInstanceId, AgentInstanceRevisionId, AuthenticatedIdentity, CreateInstanceUpdate, Digest,
    ParameterDocument, Postgres, ReleaseAgentId, ReleaseServiceError, ReviseInstance,
    RevisionBindingRow, RuntimePolicy, Sha256, Transaction, Uuid, Value, json,
};

// Revision fields stay explicit to prevent accidental runtime-policy omission.
#[allow(clippy::too_many_arguments)]
pub async fn insert_revision(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: AgentInstanceRevisionId,
    instance_id: AgentInstanceId,
    release_agent_id: ReleaseAgentId,
    parameters: &ParameterDocument,
    selection: &RuntimePolicy,
    effective: &RuntimePolicy,
    platform_policy_version: &str,
    publication_mode: &str,
    publication_repository_binding_id: Option<Uuid>,
    runnable: bool,
    diagnostics: &[Value],
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    let effective_bytes = serde_json::to_vec(effective)?;
    let effective_hash: [u8; 32] = Sha256::digest(&effective_bytes).into();
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          secret_bindings, resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, publication_mode,
          publication_repository_binding_id, runnable, diagnostics, created_by)
         VALUES ($1, $2, $3, $4, $5, '[]', $6, $7, $8, $9,
                 $10, $11, $12, $13, $14, $15)",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(serde_json::to_value(parameters.values())?)
    .bind(parameters.hash().as_bytes().as_slice())
    .bind(serde_json::to_value(selection)?)
    .bind(json!({"network": selection.network}))
    .bind(serde_json::to_value(effective)?)
    .bind(effective_hash.as_slice())
    .bind(platform_policy_version)
    .bind(publication_mode)
    .bind(publication_repository_binding_id)
    .bind(runnable)
    .bind(serde_json::to_value(diagnostics)?)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_revision_with_binding_ids(
    tx: &mut Transaction<'_, Postgres>,
    command: &ReviseInstance,
    release_agent_id: Uuid,
    parameters: &ParameterDocument,
    effective: &RuntimePolicy,
    publication_mode: &str,
    publication_repository_binding_id: Option<Uuid>,
    runnable: bool,
    diagnostics: &[Value],
    binding_ids: &[Uuid],
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    let effective_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(effective)?).into();
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          secret_bindings, resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, publication_mode,
          publication_repository_binding_id, runnable, diagnostics, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16)",
    )
    .bind(command.new_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(release_agent_id)
    .bind(serde_json::to_value(parameters.values())?)
    .bind(parameters.hash().as_bytes().as_slice())
    .bind(serde_json::to_value(binding_ids)?)
    .bind(serde_json::to_value(&command.selected_policy)?)
    .bind(json!({"network": command.selected_policy.network}))
    .bind(serde_json::to_value(effective)?)
    .bind(effective_hash.as_slice())
    .bind(&command.platform_policy_version)
    .bind(publication_mode)
    .bind(publication_repository_binding_id)
    .bind(runnable)
    .bind(serde_json::to_value(diagnostics)?)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_update_candidate_revision(
    tx: &mut Transaction<'_, Postgres>,
    command: &CreateInstanceUpdate,
    parameters: &ParameterDocument,
    effective: &RuntimePolicy,
    publication_mode: &str,
    publication_repository_binding_id: Option<Uuid>,
    runnable: bool,
    diagnostics: &[Value],
    binding_ids: &[Uuid],
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    let effective_hash: [u8; 32] = Sha256::digest(serde_json::to_vec(effective)?).into();
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          secret_bindings, resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, publication_mode,
          publication_repository_binding_id, runnable, diagnostics, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16)",
    )
    .bind(command.candidate_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.candidate_release_agent_id.as_uuid())
    .bind(serde_json::to_value(parameters.values())?)
    .bind(parameters.hash().as_bytes().as_slice())
    .bind(serde_json::to_value(binding_ids)?)
    .bind(serde_json::to_value(&command.selected_policy)?)
    .bind(json!({"network": command.selected_policy.network}))
    .bind(serde_json::to_value(effective)?)
    .bind(effective_hash.as_slice())
    .bind(&command.platform_policy_version)
    .bind(publication_mode)
    .bind(publication_repository_binding_id)
    .bind(runnable)
    .bind(serde_json::to_value(diagnostics)?)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn clone_revision_binding(
    tx: &mut Transaction<'_, Postgres>,
    binding_id: Uuid,
    revision_id: AgentInstanceRevisionId,
    binding: &RevisionBindingRow,
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO agent_secret_bindings
         (id, instance_revision_id, import_id, slot_key, delivery_mode,
          phases, attachment_ids, destinations, effective_policy,
          effective_policy_hash, status, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 'active', $11)",
    )
    .bind(binding_id)
    .bind(revision_id.as_uuid())
    .bind(binding.import_id)
    .bind(&binding.slot_key)
    .bind(&binding.delivery_mode)
    .bind(&binding.phases)
    .bind(&binding.attachment_ids)
    .bind(&binding.destinations)
    .bind(&binding.effective_policy)
    .bind(&binding.effective_policy_hash)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}
