use super::{
    AgentInstanceRevisionId, AuthenticatedIdentity, BoundGitCapability, BranchUpdatePolicy,
    CapabilityBinding, CapabilityOperation, CapabilityRequirement, CapabilityRequirementId,
    CapabilityRevisionDiagnostic, CapabilitySlotKey, ChangedPathGlob, Digest, GitCapabilityCeiling,
    GitOperation, Postgres, RefGlob, RefMutationPermission, ReleaseAgentId, ReleaseServiceError,
    Sha256, Transaction, Uuid, Value,
};

pub async fn insert_capability_binding(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: AgentInstanceRevisionId,
    release_agent_id: Uuid,
    binding: &CapabilityBinding,
    authorization_model_version: &str,
    identity: &AuthenticatedIdentity,
) -> Result<(), ReleaseServiceError> {
    sqlx::query(
        "INSERT INTO agent_capability_bindings
         (id, instance_revision_id, release_agent_id, requirement_id,
          requirement_hash, slot_key, resource_kind, resource_id,
          granted_operations, normalized_hash, authorization_model_version,
          created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(binding.id().as_uuid())
    .bind(revision_id.as_uuid())
    .bind(release_agent_id)
    .bind(binding.requirement_id().as_uuid())
    .bind(binding.requirement_hash().as_bytes().as_slice())
    .bind(binding.slot().as_str())
    .bind(binding.resource().kind.as_str())
    .bind(binding.resource().id)
    .bind(operation_names(binding.granted_operations()))
    .bind(binding.normalized_hash().as_bytes().as_slice())
    .bind(authorization_model_version)
    .bind(identity.user_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_release_git_ceiling(
    tx: &mut Transaction<'_, Postgres>,
    release_agent_id: ReleaseAgentId,
    requirement_id: CapabilityRequirementId,
    ceiling: &GitCapabilityCeiling,
) -> Result<(), ReleaseServiceError> {
    let policy = ceiling.update_policy();
    let limits = ceiling.transfer_limits();
    sqlx::query(
        "INSERT INTO release_git_capability_ceilings
         (requirement_id, release_agent_id, grammar_version, git_operations,
          ref_globs, changed_path_globs, branch_update_policy,
          branch_create, branch_delete, tag_create, tag_update, tag_delete,
          other_create, other_update, other_delete, request_bytes, pack_bytes,
          object_count, ref_updates, exact_parent_required, normalized_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 $13, $14, $15, $16, $17, $18, $19, $20, $21)",
    )
    .bind(requirement_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(i16::try_from(ceiling.version()).map_err(|_| ReleaseServiceError::InvalidStoredData)?)
    .bind(git_operation_names(ceiling.operations()))
    .bind(
        ceiling
            .ref_globs()
            .iter()
            .map(RefGlob::as_str)
            .collect::<Vec<_>>(),
    )
    .bind(
        ceiling
            .changed_path_globs()
            .iter()
            .map(ChangedPathGlob::as_str)
            .collect::<Vec<_>>(),
    )
    .bind(branch_update_name(policy.branches.updates))
    .bind(permission_allowed(policy.branches.create))
    .bind(permission_allowed(policy.branches.delete))
    .bind(permission_allowed(policy.tags.create))
    .bind(permission_allowed(policy.tags.update))
    .bind(permission_allowed(policy.tags.delete))
    .bind(permission_allowed(policy.other.create))
    .bind(permission_allowed(policy.other.update))
    .bind(permission_allowed(policy.other.delete))
    .bind(
        i64::try_from(limits.request_bytes())
            .map_err(|_| ReleaseServiceError::InvalidStoredData)?,
    )
    .bind(i64::try_from(limits.pack_bytes()).map_err(|_| ReleaseServiceError::InvalidStoredData)?)
    .bind(i32::try_from(limits.object_count()).map_err(|_| ReleaseServiceError::InvalidStoredData)?)
    .bind(i32::from(limits.ref_updates()))
    .bind(ceiling.exact_parent_required())
    .bind(ceiling.normalized_hash()?.as_bytes().as_slice())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_git_capability_binding(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: AgentInstanceRevisionId,
    generic_binding: &CapabilityBinding,
    git_binding: &BoundGitCapability,
) -> Result<(), ReleaseServiceError> {
    let authority = git_binding.authority();
    let policy = authority.update_policy();
    let limits = authority.transfer_limits();
    sqlx::query(
        "INSERT INTO agent_git_capability_bindings
         (binding_id, instance_revision_id, requirement_id, grammar_version,
          git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, normalized_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 $13, $14, $15, $16, $17, $18, $19, $20, $21, $22)",
    )
    .bind(generic_binding.id().as_uuid())
    .bind(revision_id.as_uuid())
    .bind(generic_binding.requirement_id().as_uuid())
    .bind(i16::try_from(authority.version()).map_err(|_| ReleaseServiceError::InvalidStoredData)?)
    .bind(git_operation_names(authority.operations()))
    .bind(
        authority
            .ref_globs()
            .iter()
            .map(RefGlob::as_str)
            .collect::<Vec<_>>(),
    )
    .bind(
        authority
            .changed_path_globs()
            .iter()
            .map(ChangedPathGlob::as_str)
            .collect::<Vec<_>>(),
    )
    .bind(branch_update_name(policy.branches.updates))
    .bind(permission_allowed(policy.branches.create))
    .bind(permission_allowed(policy.branches.delete))
    .bind(permission_allowed(policy.tags.create))
    .bind(permission_allowed(policy.tags.update))
    .bind(permission_allowed(policy.tags.delete))
    .bind(permission_allowed(policy.other.create))
    .bind(permission_allowed(policy.other.update))
    .bind(permission_allowed(policy.other.delete))
    .bind(
        i64::try_from(limits.request_bytes())
            .map_err(|_| ReleaseServiceError::InvalidStoredData)?,
    )
    .bind(i64::try_from(limits.pack_bytes()).map_err(|_| ReleaseServiceError::InvalidStoredData)?)
    .bind(i32::try_from(limits.object_count()).map_err(|_| ReleaseServiceError::InvalidStoredData)?)
    .bind(i32::from(limits.ref_updates()))
    .bind(authority.exact_parent_required())
    .bind(git_binding.normalized_hash()?.as_bytes().as_slice())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub const fn git_operation_name(operation: GitOperation) -> &'static str {
    match operation {
        GitOperation::Discover => "discover",
        GitOperation::Fetch => "fetch",
        GitOperation::Receive => "receive",
    }
}

pub fn git_operation_names(operations: &[GitOperation]) -> Vec<&'static str> {
    operations.iter().copied().map(git_operation_name).collect()
}

pub const fn branch_update_name(policy: BranchUpdatePolicy) -> &'static str {
    match policy {
        BranchUpdatePolicy::FastForwardOnly => "fast_forward_only",
        BranchUpdatePolicy::AllowForce => "allow_force",
    }
}

pub const fn permission_allowed(permission: RefMutationPermission) -> bool {
    matches!(permission, RefMutationPermission::Allow)
}

pub fn capability_diagnostics_from_json(
    diagnostics: &Value,
) -> Result<Vec<CapabilityRevisionDiagnostic>, ReleaseServiceError> {
    diagnostics
        .as_array()
        .ok_or(ReleaseServiceError::InvalidStoredData)?
        .iter()
        .filter(|value| {
            value.get("code").and_then(Value::as_str) == Some("required_capability_binding_missing")
        })
        .map(|value| {
            let field = value
                .get("field")
                .and_then(Value::as_str)
                .and_then(|field| field.strip_prefix("capability_slots."))
                .ok_or(ReleaseServiceError::InvalidStoredData)?;
            Ok(CapabilityRevisionDiagnostic {
                code: String::from("required_capability_binding_missing"),
                slot: CapabilitySlotKey::parse(field)?,
            })
        })
        .collect()
}

pub fn release_capability_requirements(
    release_agent_id: ReleaseAgentId,
    declarations: &[agent_config::CapabilitySlotDeclaration],
) -> Result<Vec<(String, CapabilityRequirement)>, ReleaseServiceError> {
    let mut declarations = declarations.iter().collect::<Vec<_>>();
    declarations.sort_unstable_by(|left, right| left.key.cmp(&right.key));
    declarations
        .into_iter()
        .map(|declaration| {
            let id = deterministic_requirement_id(release_agent_id, &declaration.key);
            declaration
                .to_requirement(id)
                .map(|requirement| (declaration.purpose.clone(), requirement))
                .map_err(ReleaseServiceError::Capability)
        })
        .collect()
}

pub fn deterministic_requirement_id(
    release_agent_id: ReleaseAgentId,
    slot: &str,
) -> CapabilityRequirementId {
    let mut hasher = Sha256::new();
    hasher.update(b"hephaestus.release-capability-requirement-id.v1");
    hasher.update(release_agent_id.as_uuid().as_bytes());
    hasher.update(slot.len().to_be_bytes());
    hasher.update(slot.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 UUIDv8 marks this deterministic application-defined UUID.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    CapabilityRequirementId::from_uuid(Uuid::from_bytes(bytes))
}

pub fn release_capability_requirements_hash(
    requirements: &[(String, CapabilityRequirement)],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"hephaestus.release-capability-requirements.v1");
    hasher.update(requirements.len().to_be_bytes());
    for (purpose, requirement) in requirements {
        hasher.update(requirement.slot().as_str().len().to_be_bytes());
        hasher.update(requirement.slot().as_str().as_bytes());
        hasher.update(purpose.len().to_be_bytes());
        hasher.update(purpose.as_bytes());
        hasher.update(requirement.normalized_hash().as_bytes());
    }
    hasher.finalize().into()
}

pub fn operation_names(
    operations: impl IntoIterator<Item = CapabilityOperation>,
) -> Vec<&'static str> {
    operations
        .into_iter()
        .map(CapabilityOperation::as_str)
        .collect()
}
