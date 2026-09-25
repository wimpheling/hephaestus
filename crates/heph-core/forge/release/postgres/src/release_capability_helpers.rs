use super::{
    AgentInstanceRevisionId, BTreeMap, BTreeSet, BoundGitCapability, BranchRefPolicy,
    BranchUpdatePolicy, CapabilityBinding, CapabilityBindingId, CapabilityOperation,
    CapabilityRequirement, CapabilityRequirementId, CapabilityRequirementRow, CapabilityResource,
    CapabilitySlotKey, CarriedCapabilityBinding, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitCapabilityRow, GitOperation, GitRepositoryId, Postgres, RefGlob,
    RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy, ReleaseServiceError,
    StoredCapabilityBindingRow, Transaction, TransferLimits, Uuid, parse_capability_operation,
    parse_capability_resource_kind, permission_allowed,
};

pub fn stored_requirement(
    row: CapabilityRequirementRow,
) -> Result<(CapabilitySlotKey, CapabilityRequirement), ReleaseServiceError> {
    let slot = CapabilitySlotKey::parse(row.slot_key)?;
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(row.id),
        slot.clone(),
        parse_capability_resource_kind(&row.resource_kind)?,
        row.required_operations
            .iter()
            .map(String::as_str)
            .map(parse_capability_operation)
            .collect::<Result<Vec<_>, _>>()?,
        row.optional_operations
            .iter()
            .map(String::as_str)
            .map(parse_capability_operation)
            .collect::<Result<Vec<_>, _>>()?,
        row.slot_required,
    )?;
    if row.normalized_hash.as_slice() != requirement.normalized_hash().as_bytes() {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    Ok((slot, requirement))
}

pub async fn load_capability_requirements(
    tx: &mut Transaction<'_, Postgres>,
    release_agent_id: Uuid,
) -> Result<BTreeMap<CapabilitySlotKey, CapabilityRequirement>, ReleaseServiceError> {
    let rows: Vec<CapabilityRequirementRow> = sqlx::query_as(
        "SELECT id, slot_key, resource_kind, required_operations,
                optional_operations, slot_required, normalized_hash
         FROM release_capability_requirements
         WHERE release_agent_id = $1
         ORDER BY slot_key, id",
    )
    .bind(release_agent_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(stored_requirement).collect()
}

pub async fn load_git_ceilings(
    tx: &mut Transaction<'_, Postgres>,
    release_agent_id: Uuid,
) -> Result<BTreeMap<CapabilityRequirementId, GitCapabilityCeiling>, ReleaseServiceError> {
    let rows = sqlx::query_as::<_, GitCapabilityRow>(
        "SELECT requirement_id, grammar_version, git_operations, ref_globs,
                changed_path_globs, branch_update_policy, branch_create,
                branch_delete, tag_create, tag_update, tag_delete,
                other_create, other_update, other_delete, request_bytes,
                pack_bytes, object_count, ref_updates, exact_parent_required,
                normalized_hash
         FROM release_git_capability_ceilings
         WHERE release_agent_id = $1
         ORDER BY requirement_id",
    )
    .bind(release_agent_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let requirement_id = CapabilityRequirementId::from_uuid(row.requirement_id);
            let ceiling = stored_git_ceiling(&row)?;
            Ok((requirement_id, ceiling))
        })
        .collect()
}

pub fn stored_git_ceiling(
    row: &GitCapabilityRow,
) -> Result<GitCapabilityCeiling, ReleaseServiceError> {
    let version =
        u16::try_from(row.grammar_version).map_err(|_| ReleaseServiceError::InvalidStoredData)?;
    let ceiling = git_ceiling_from_row(row)?;
    if version != ceiling.version()
        || row.normalized_hash.as_slice() != ceiling.normalized_hash()?.as_bytes()
    {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    Ok(ceiling)
}

pub fn git_ceiling_from_row(
    row: &GitCapabilityRow,
) -> Result<GitCapabilityCeiling, ReleaseServiceError> {
    GitCapabilityCeiling::new(GitCapabilityCeilingInput {
        operations: row
            .git_operations
            .iter()
            .map(|operation| match operation.as_str() {
                "discover" => Ok(GitOperation::Discover),
                "fetch" => Ok(GitOperation::Fetch),
                "receive" => Ok(GitOperation::Receive),
                _ => Err(ReleaseServiceError::InvalidStoredData),
            })
            .collect::<Result<_, _>>()?,
        ref_globs: row
            .ref_globs
            .iter()
            .cloned()
            .map(RefGlob::parse_explicitly_broad)
            .collect::<Result<_, _>>()?,
        changed_path_globs: row
            .changed_path_globs
            .iter()
            .cloned()
            .map(ChangedPathGlob::parse_explicitly_broad)
            .collect::<Result<_, _>>()?,
        update_policy: RefUpdatePolicy {
            branches: BranchRefPolicy {
                updates: match row.branch_update_policy.as_str() {
                    "fast_forward_only" => BranchUpdatePolicy::FastForwardOnly,
                    "allow_force" => BranchUpdatePolicy::AllowForce,
                    _ => return Err(ReleaseServiceError::InvalidStoredData),
                },
                create: permission_from_bool(row.branch_create),
                delete: permission_from_bool(row.branch_delete),
            },
            tags: RefNamespacePolicy {
                create: permission_from_bool(row.tag_create),
                update: permission_from_bool(row.tag_update),
                delete: permission_from_bool(row.tag_delete),
            },
            other: RefNamespacePolicy {
                create: permission_from_bool(row.other_create),
                update: permission_from_bool(row.other_update),
                delete: permission_from_bool(row.other_delete),
            },
        },
        transfer_limits: TransferLimits::new(
            u64::try_from(row.request_bytes).map_err(|_| ReleaseServiceError::InvalidStoredData)?,
            u64::try_from(row.pack_bytes).map_err(|_| ReleaseServiceError::InvalidStoredData)?,
            u32::try_from(row.object_count).map_err(|_| ReleaseServiceError::InvalidStoredData)?,
            u16::try_from(row.ref_updates).map_err(|_| ReleaseServiceError::InvalidStoredData)?,
        )?,
        exact_parent_required: row.exact_parent_required,
    })
    .map_err(ReleaseServiceError::GitCapability)
}

pub const fn permission_from_bool(allowed: bool) -> RefMutationPermission {
    if allowed {
        RefMutationPermission::Allow
    } else {
        RefMutationPermission::Deny
    }
}

pub fn git_operations_for_capability<'a>(
    operations: impl IntoIterator<Item = CapabilityOperation> + 'a,
) -> Vec<GitOperation> {
    let mut read = false;
    let mut receive = false;
    for operation in operations {
        match operation {
            CapabilityOperation::GitRead => read = true,
            CapabilityOperation::CreateRef
            | CapabilityOperation::UpdateRef
            | CapabilityOperation::ForceUpdateRef
            | CapabilityOperation::DeleteRef
            | CapabilityOperation::CreateTag
            | CapabilityOperation::DeleteTag => receive = true,
            _ => {}
        }
    }
    let mut values = Vec::with_capacity(3);
    if read {
        values.extend([GitOperation::Discover, GitOperation::Fetch]);
    }
    if receive {
        values.push(GitOperation::Receive);
    }
    values
}

pub fn git_authority_matches_capability(
    authority: &GitCapabilityCeiling,
    binding: &CapabilityBinding,
) -> bool {
    let granted = binding.granted_operations().collect::<BTreeSet<_>>();
    if authority.operations() != git_operations_for_capability(granted.iter().copied()).as_slice() {
        return false;
    }
    if !authority.operations().contains(&GitOperation::Receive) {
        return true;
    }
    let policy = authority.update_policy();
    granted.contains(&CapabilityOperation::UpdateRef)
        && (policy.branches.updates != BranchUpdatePolicy::AllowForce
            || granted.contains(&CapabilityOperation::ForceUpdateRef))
        && (!permission_allowed(policy.branches.create)
            || granted.contains(&CapabilityOperation::CreateRef))
        && (!permission_allowed(policy.branches.delete)
            || granted.contains(&CapabilityOperation::DeleteRef))
        && (!permission_allowed(policy.tags.create)
            || granted.contains(&CapabilityOperation::CreateTag))
        && (!permission_allowed(policy.tags.update)
            || granted.contains(&CapabilityOperation::ForceUpdateRef))
        && (!permission_allowed(policy.tags.delete)
            || granted.contains(&CapabilityOperation::DeleteTag))
        && (!permission_allowed(policy.other.create)
            || granted.contains(&CapabilityOperation::CreateRef))
        && (!permission_allowed(policy.other.update)
            || granted.contains(&CapabilityOperation::ForceUpdateRef))
        && (!permission_allowed(policy.other.delete)
            || granted.contains(&CapabilityOperation::DeleteRef))
}

pub async fn load_carried_capability_bindings(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: AgentInstanceRevisionId,
) -> Result<Vec<CarriedCapabilityBinding>, ReleaseServiceError> {
    let rows: Vec<StoredCapabilityBindingRow> = sqlx::query_as(
        "SELECT binding.id AS binding_id,
                requirement.id AS requirement_id,
                requirement.slot_key,
                requirement.resource_kind AS requirement_resource_kind,
                requirement.required_operations,
                requirement.optional_operations,
                requirement.slot_required,
                requirement.normalized_hash AS requirement_normalized_hash,
                binding.requirement_hash AS binding_requirement_hash,
                binding.resource_kind AS binding_resource_kind,
                binding.resource_id,
                binding.granted_operations,
                binding.normalized_hash AS binding_normalized_hash,
                binding.authorization_model_version
         FROM agent_capability_bindings AS binding
         JOIN release_capability_requirements AS requirement
           ON requirement.id = binding.requirement_id
          AND requirement.release_agent_id = binding.release_agent_id
         WHERE binding.instance_revision_id = $1
         ORDER BY binding.slot_key, binding.id",
    )
    .bind(revision_id.as_uuid())
    .fetch_all(&mut **tx)
    .await?;
    let mut carried = Vec::with_capacity(rows.len());
    for row in rows {
        let (_, requirement) = stored_requirement(CapabilityRequirementRow {
            id: row.requirement_id,
            slot_key: row.slot_key,
            resource_kind: row.requirement_resource_kind,
            required_operations: row.required_operations,
            optional_operations: row.optional_operations,
            slot_required: row.slot_required,
            normalized_hash: row.requirement_normalized_hash,
        })?;
        if row.binding_requirement_hash.as_slice() != requirement.normalized_hash().as_bytes() {
            return Err(ReleaseServiceError::InvalidStoredData);
        }
        let resource_kind = parse_capability_resource_kind(&row.binding_resource_kind)?;
        let granted_operations = row
            .granted_operations
            .iter()
            .map(String::as_str)
            .map(parse_capability_operation)
            .collect::<Result<Vec<_>, _>>()?;
        let binding = CapabilityBinding::bind(
            CapabilityBindingId::from_uuid(row.binding_id),
            &requirement,
            CapabilityResource::new(resource_kind, row.resource_id),
            granted_operations,
        )?;
        if row.binding_normalized_hash.as_slice() != binding.normalized_hash().as_bytes() {
            return Err(ReleaseServiceError::InvalidStoredData);
        }
        let git_binding = load_stored_git_binding(tx, &binding).await?;
        carried.push(CarriedCapabilityBinding {
            requirement,
            binding,
            git_binding,
            authorization_model_version: row.authorization_model_version,
        });
    }
    Ok(carried)
}

pub async fn load_stored_git_binding(
    tx: &mut Transaction<'_, Postgres>,
    binding: &CapabilityBinding,
) -> Result<Option<BoundGitCapability>, ReleaseServiceError> {
    let row = sqlx::query_as::<_, GitCapabilityRow>(
        "SELECT requirement_id, grammar_version, git_operations, ref_globs,
                changed_path_globs, branch_update_policy, branch_create,
                branch_delete, tag_create, tag_update, tag_delete,
                other_create, other_update, other_delete, request_bytes,
                pack_bytes, object_count, ref_updates, exact_parent_required,
                normalized_hash
         FROM agent_git_capability_bindings WHERE binding_id = $1",
    )
    .bind(binding.id().as_uuid())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.requirement_id != binding.requirement_id().as_uuid() {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    let ceiling_row = sqlx::query_as::<_, GitCapabilityRow>(
        "SELECT requirement_id, grammar_version, git_operations, ref_globs,
                changed_path_globs, branch_update_policy, branch_create,
                branch_delete, tag_create, tag_update, tag_delete,
                other_create, other_update, other_delete, request_bytes,
                pack_bytes, object_count, ref_updates, exact_parent_required,
                normalized_hash
         FROM release_git_capability_ceilings WHERE requirement_id = $1",
    )
    .bind(binding.requirement_id().as_uuid())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ReleaseServiceError::InvalidStoredData)?;
    let release_ceiling = stored_git_ceiling(&ceiling_row)?;
    let authority = git_ceiling_from_row(&row)?;
    if u16::try_from(row.grammar_version).ok() != Some(authority.version()) {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    let bound = BoundGitCapability::new(
        GitRepositoryId::new(binding.resource().id),
        authority,
        &release_ceiling,
    )?;
    if row.normalized_hash.as_slice() != bound.normalized_hash()?.as_bytes() {
        return Err(ReleaseServiceError::InvalidStoredData);
    }
    Ok(Some(bound))
}
