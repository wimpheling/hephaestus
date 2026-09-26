use super::*;

#[derive(sqlx::FromRow)]
pub(super) struct RevisionCloneRow {
    pub(super) project_id: Uuid,
    pub(super) active_revision_id: Option<Uuid>,
    pub(super) release_agent_id: Uuid,
    pub(super) parameters: serde_json::Value,
    pub(super) parameter_hash: Vec<u8>,
    pub(super) resource_selection: serde_json::Value,
    pub(super) network_restriction: serde_json::Value,
    pub(super) effective_runtime_policy: serde_json::Value,
    pub(super) effective_policy_hash: Vec<u8>,
    pub(super) platform_policy_version: String,
    pub(super) publication_repository_binding_id: Option<Uuid>,
    pub(super) secret_slot_schema: serde_json::Value,
}

#[derive(sqlx::FromRow)]
pub(super) struct CarriedCapabilityRow {
    pub(super) source_binding_id: Uuid,
    pub(super) requirement_id: Uuid,
    pub(super) slot_key: String,
    pub(super) requirement_resource_kind: String,
    pub(super) required_operations: Vec<String>,
    pub(super) optional_operations: Vec<String>,
    pub(super) slot_required: bool,
    pub(super) resource_kind: String,
    pub(super) resource_id: Uuid,
    pub(super) granted_operations: Vec<String>,
    pub(super) authorization_model_version: String,
}

pub(super) fn clone_capability_binding(
    row: &CarriedCapabilityRow,
) -> Result<CapabilityBinding, SecretServiceError> {
    let requirement_kind = parse_capability_resource_kind(&row.requirement_resource_kind)?;
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(row.requirement_id),
        CapabilitySlotKey::parse(row.slot_key.clone())
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        requirement_kind,
        row.required_operations
            .iter()
            .map(|operation| parse_capability_operation(operation))
            .collect::<Result<Vec<_>, _>>()?,
        row.optional_operations
            .iter()
            .map(|operation| parse_capability_operation(operation))
            .collect::<Result<Vec<_>, _>>()?,
        row.slot_required,
    )
    .map_err(|_| SecretServiceError::InvalidStoredData)?;
    CapabilityBinding::bind(
        CapabilityBindingId::new(),
        &requirement,
        CapabilityResource::new(
            parse_capability_resource_kind(&row.resource_kind)?,
            row.resource_id,
        ),
        row.granted_operations
            .iter()
            .map(|operation| parse_capability_operation(operation))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|_| SecretServiceError::InvalidStoredData)
}

pub(super) fn parse_capability_resource_kind(
    value: &str,
) -> Result<CapabilityResourceKind, SecretServiceError> {
    match value {
        "repository" => Ok(CapabilityResourceKind::Repository),
        "project" => Ok(CapabilityResourceKind::Project),
        "agent_instance" => Ok(CapabilityResourceKind::AgentInstance),
        "gateway" => Ok(CapabilityResourceKind::Gateway),
        "run" => Ok(CapabilityResourceKind::Run),
        "state_volume" => Ok(CapabilityResourceKind::StateVolume),
        _ => Err(SecretServiceError::InvalidStoredData),
    }
}

pub(super) fn parse_capability_operation(
    value: &str,
) -> Result<CapabilityOperation, SecretServiceError> {
    match value {
        "inspect" => Ok(CapabilityOperation::Inspect),
        "configure" => Ok(CapabilityOperation::Configure),
        "execute" => Ok(CapabilityOperation::Execute),
        "update" => Ok(CapabilityOperation::Update),
        "pause" => Ok(CapabilityOperation::Pause),
        "recover" => Ok(CapabilityOperation::Recover),
        "cancel" => Ok(CapabilityOperation::Cancel),
        "attach" => Ok(CapabilityOperation::Attach),
        "restore" => Ok(CapabilityOperation::Restore),
        "git_read" => Ok(CapabilityOperation::GitRead),
        "create_ref" => Ok(CapabilityOperation::CreateRef),
        "update_ref" => Ok(CapabilityOperation::UpdateRef),
        "force_update_ref" => Ok(CapabilityOperation::ForceUpdateRef),
        "delete_ref" => Ok(CapabilityOperation::DeleteRef),
        "create_tag" => Ok(CapabilityOperation::CreateTag),
        "delete_tag" => Ok(CapabilityOperation::DeleteTag),
        "trigger_run" => Ok(CapabilityOperation::TriggerRun),
        "manage_attachments" => Ok(CapabilityOperation::ManageAttachments),
        _ => Err(SecretServiceError::InvalidStoredData),
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct EligibleImportRow {
    pub(super) grant_id: Uuid,
    pub(super) secret_id: Uuid,
    pub(super) owner_organization_id: Uuid,
    pub(super) target_kind: String,
    pub(super) target_id: Uuid,
    pub(super) delivery_modes: Vec<String>,
    pub(super) phases: Vec<String>,
    pub(super) destinations: Vec<String>,
}

#[derive(sqlx::FromRow)]
pub(super) struct BrokeredRuleBindingRow {
    pub(super) instance_id: Uuid,
    pub(super) instance_revision_id: Uuid,
    pub(super) import_id: Uuid,
    pub(super) delivery_mode: String,
    pub(super) destinations: Vec<String>,
    pub(super) secret_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub(super) struct CarriedBindingRow {
    pub(super) import_id: Uuid,
    pub(super) slot_key: String,
    pub(super) delivery_mode: String,
    pub(super) phases: Vec<String>,
    pub(super) attachment_ids: Vec<Uuid>,
    pub(super) destinations: Vec<String>,
    pub(super) effective_policy: serde_json::Value,
    pub(super) effective_policy_hash: Vec<u8>,
}
