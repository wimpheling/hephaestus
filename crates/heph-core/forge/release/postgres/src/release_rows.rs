use super::{
    AgentUpdateId, BoundGitCapability, CapabilityBinding, CapabilityRequirement, OffsetDateTime,
    Postgres, ReleaseServiceError, Transaction, Uuid, Value, append_event, json,
};

#[derive(sqlx::FromRow)]
pub struct BuildRow {
    pub repository_id: Uuid,
    pub source_commit: String,
    pub source_ref: String,
    pub build_definition_hash: Vec<u8>,
    pub guest_image_reference: Option<String>,
    pub state: String,
}

#[derive(sqlx::FromRow)]
pub struct ReleaseAgentRow {
    pub family_id: Uuid,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub requires_state: bool,
    pub publication_mode: String,
}

#[derive(sqlx::FromRow)]
pub struct RevisionUpdateRow {
    pub active_revision_id: Option<Uuid>,
    pub project_id: Uuid,
    pub release_agent_id: Uuid,
    pub secret_bindings: Value,
    pub publication_mode: String,
    pub publication_repository_slot: Option<String>,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
}

#[derive(sqlx::FromRow)]
pub struct RevisionBindingRow {
    pub id: Uuid,
    pub import_id: Uuid,
    pub slot_key: String,
    pub delivery_mode: String,
    pub phases: Vec<String>,
    pub attachment_ids: Vec<Uuid>,
    pub destinations: Vec<String>,
    pub effective_policy: Value,
    pub effective_policy_hash: Vec<u8>,
}

#[derive(sqlx::FromRow)]
pub struct BrokeredRuleCloneRow {
    pub id: Uuid,
    pub binding_id: Uuid,
    pub secret_version_id: Uuid,
    pub destination_origin: String,
    pub location_kind: String,
    pub header_name: String,
    pub header_prefix: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct CapabilityRevisionSourceRow {
    pub active_revision_id: Option<Uuid>,
    pub project_id: Uuid,
    pub release_agent_id: Uuid,
    pub secret_bindings: Value,
    pub diagnostics: Value,
    pub publication_repository_slot: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct CapabilityRequirementRow {
    pub id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub required_operations: Vec<String>,
    pub optional_operations: Vec<String>,
    pub slot_required: bool,
    pub normalized_hash: Vec<u8>,
}

// SQL rows mirror independently attenuable transition flags from the typed
// persistence schema; grouping them would weaken the column-to-domain audit.
#[allow(clippy::struct_excessive_bools)]
#[derive(sqlx::FromRow)]
pub struct GitCapabilityRow {
    pub requirement_id: Uuid,
    pub grammar_version: i16,
    pub git_operations: Vec<String>,
    pub ref_globs: Vec<String>,
    pub changed_path_globs: Vec<String>,
    pub branch_update_policy: String,
    pub branch_create: bool,
    pub branch_delete: bool,
    pub tag_create: bool,
    pub tag_update: bool,
    pub tag_delete: bool,
    pub other_create: bool,
    pub other_update: bool,
    pub other_delete: bool,
    pub request_bytes: i64,
    pub pack_bytes: i64,
    pub object_count: i32,
    pub ref_updates: i32,
    pub exact_parent_required: bool,
    pub normalized_hash: Vec<u8>,
}

#[derive(sqlx::FromRow)]
pub struct StoredCapabilityBindingRow {
    pub binding_id: Uuid,
    pub requirement_id: Uuid,
    pub slot_key: String,
    pub requirement_resource_kind: String,
    pub required_operations: Vec<String>,
    pub optional_operations: Vec<String>,
    pub slot_required: bool,
    pub requirement_normalized_hash: Vec<u8>,
    pub binding_requirement_hash: Vec<u8>,
    pub binding_resource_kind: String,
    pub resource_id: Uuid,
    pub granted_operations: Vec<String>,
    pub binding_normalized_hash: Vec<u8>,
    pub authorization_model_version: String,
}

pub struct CarriedCapabilityBinding {
    pub requirement: CapabilityRequirement,
    pub binding: CapabilityBinding,
    pub git_binding: Option<BoundGitCapability>,
    pub authorization_model_version: String,
}

#[derive(sqlx::FromRow)]
pub struct UpdateCurrentRow {
    pub active_revision_id: Option<Uuid>,
    pub family_id: Uuid,
    pub project_id: Uuid,
    pub state: String,
    pub run_gate_open: bool,
    pub requires_state: bool,
    pub secret_bindings: Value,
}

#[derive(sqlx::FromRow)]
pub struct UpdateRecoveryRow {
    pub instance_id: Uuid,
    pub expected_current_revision_id: Uuid,
    pub candidate_revision_id: Uuid,
    pub state: String,
    pub active_revision_id: Option<Uuid>,
    pub instance_state: String,
    pub run_gate_open: bool,
}

#[derive(sqlx::FromRow)]
pub struct UpdateCandidateRow {
    pub family_id: Uuid,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub requires_state: bool,
    pub update_hook: Option<Value>,
    pub publication_mode: String,
    pub publication_repository_slot: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct UpdateLifecycleRow {
    pub instance_id: Uuid,
    pub expected_current_revision_id: Uuid,
    pub candidate_revision_id: Uuid,
    pub state: String,
}

pub async fn append_rejected_update_event(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    update: &UpdateLifecycleRow,
    exit_code: i32,
) -> Result<(), ReleaseServiceError> {
    append_event(
        tx,
        update_id.as_uuid(),
        "hephaestus.agent_update.rejected.v1",
        "agent_update.rejected.v1",
        json!({
            "update_id": update_id,
            "instance_id": update.instance_id,
            "expected_revision_id": update.expected_current_revision_id,
            "candidate_revision_id": update.candidate_revision_id,
            "reason": "hook_exit_nonzero",
            "exit_code": exit_code,
        }),
    )
    .await
}

pub async fn append_uncertain_update_events(
    tx: &mut Transaction<'_, Postgres>,
    update_id: AgentUpdateId,
    update: &UpdateLifecycleRow,
    reason: &str,
    paused_state: &str,
) -> Result<(), ReleaseServiceError> {
    append_event(
        tx,
        update_id.as_uuid(),
        "hephaestus.agent_update.uncertain.v1",
        "agent_update.uncertain.v1",
        json!({
            "update_id": update_id,
            "instance_id": update.instance_id,
            "expected_revision_id": update.expected_current_revision_id,
            "candidate_revision_id": update.candidate_revision_id,
            "reason": reason,
        }),
    )
    .await?;
    append_event(
        tx,
        update.instance_id,
        "hephaestus.agent_instance.paused.v1",
        "agent_instance.paused.v1",
        json!({
            "instance_id": update.instance_id,
            "update_id": update_id,
            "state": paused_state,
        }),
    )
    .await
}

#[derive(sqlx::FromRow)]
pub struct UpdateRunResultRow {
    pub update_id: Uuid,
    pub update_state: String,
    pub run_state: String,
    pub outcome: Option<String>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub failure: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct UpdateHookAdmissionRow {
    pub instance_id: Uuid,
    pub candidate_revision_id: Uuid,
    pub state: String,
    pub instance_state: String,
    pub created_at: OffsetDateTime,
    pub release_agent_id: Uuid,
    pub release_id: Uuid,
    pub requires_state: bool,
}

#[derive(sqlx::FromRow)]
pub struct DeferredMaterializationRow {
    pub id: Uuid,
    pub attachment_id: Uuid,
    pub repository_id: Uuid,
    pub target_ref: String,
    pub target_commit: String,
    pub source_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub platform_policy_version: String,
    pub requires_state: bool,
}
