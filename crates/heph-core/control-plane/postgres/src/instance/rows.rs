use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum InstanceQueryError {
    #[error("agent-instance access is denied")]
    PermissionDenied,
    #[error("agent instance was not found")]
    NotFound,
    #[error("agent-instance response exceeds its bounded collection limits")]
    ResponseTooLarge,
    #[error("agent-instance query failed")]
    Persistence(#[source] sqlx::Error),
}

#[derive(FromRow)]
// These independent flags are database-projected authorization decisions.
#[allow(clippy::struct_excessive_bools)]
pub struct InstanceRow {
    pub id: Uuid,
    pub name: String,
    pub state: String,
    pub run_gate_open: bool,
    pub active_revision_id: Uuid,
    pub state_volume_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub can_manage: bool,
    pub can_update: bool,
    pub can_recover: bool,
}

#[derive(FromRow)]
pub struct RevisionRow {
    pub id: Uuid,
    pub parameters: Value,
    pub parameter_hash: Vec<u8>,
    pub resource_selection: Value,
    pub network_restriction: Value,
    pub effective_runtime_policy: Value,
    pub platform_policy_version: String,
    pub runnable: bool,
    pub diagnostics: Value,
    pub created_at: OffsetDateTime,
    pub release_agent_id: Uuid,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub update_hook: Option<Value>,
    pub release_id: Uuid,
    pub release_version: String,
    pub release_state: String,
    pub release_agent_name: String,
}

#[derive(FromRow)]
pub struct AttachmentRow {
    pub id: Uuid,
    pub ref_selector: String,
    pub trigger_policy: String,
    pub enabled: bool,
    pub removed_at: Option<OffsetDateTime>,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub can_manage: bool,
}

#[derive(FromRow)]
pub struct UpdateRow {
    pub id: Uuid,
    pub expected_current_revision_id: Uuid,
    pub candidate_revision_id: Uuid,
    pub state: String,
    pub hook_run_id: Option<Uuid>,
    pub hook_exit_code: Option<i32>,
    pub hook_exit_signal: Option<i32>,
    pub diagnostics: Value,
    pub final_decision: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub hook_events: Value,
}

#[derive(FromRow)]
pub struct RepositoryRow {
    pub id: Uuid,
    pub name: String,
    pub default_branch: String,
}

#[derive(FromRow)]
pub struct ImportRow {
    pub id: Uuid,
    pub alias: String,
    pub target_kind: String,
    pub target_id: Uuid,
    pub status: String,
    pub secret_name: String,
    pub secret_status: String,
    pub delivery_modes: Vec<String>,
    pub phases: Vec<String>,
    pub destinations: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
pub struct CandidateRow {
    pub id: Uuid,
    pub display_name: String,
    pub parameter_schema: Value,
    pub secret_slot_schema: Value,
    pub runtime_contract: Value,
    pub requires_state: bool,
    pub update_hook: Option<Value>,
    pub release_id: Uuid,
    pub release_version: String,
}

#[derive(FromRow)]
pub struct RecentRunRow {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub run_kind: String,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub attachment_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(FromRow)]
pub struct CapabilityRequirementRow {
    pub id: Uuid,
    pub release_agent_id: Uuid,
    pub slot_key: String,
    pub purpose: String,
    pub resource_kind: String,
    pub required_operations: Vec<String>,
    pub optional_operations: Vec<String>,
    pub slot_required: bool,
}

#[derive(FromRow)]
pub struct CapabilityResourceOptionRow {
    pub id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub display_name: String,
    pub grantable_operations: Vec<String>,
}

#[derive(FromRow)]
pub struct CapabilityBindingRow {
    pub id: Uuid,
    pub instance_revision_id: Uuid,
    pub requirement_id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub resource_id: Uuid,
    pub resource_name: String,
    pub granted_operations: Vec<String>,
    pub grantor_id: Uuid,
    pub grantor_name: String,
    pub authorization_model_version: String,
    pub created_at: OffsetDateTime,
    pub live: bool,
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
pub struct RuntimeSessionRow {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub run_id: Uuid,
    pub instance_revision_id: Uuid,
    pub status: String,
    pub issued_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
    pub acknowledged_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub revocation_reason: Option<String>,
}

#[derive(FromRow)]
pub struct CapabilityAuditRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub runtime_session_id: Uuid,
    pub snapshot_id: Uuid,
    pub binding_id: Uuid,
    pub slot_key: String,
    pub resource_kind: String,
    pub resource_id: Uuid,
    pub operation: String,
    pub event_kind: String,
    pub decision: Option<String>,
    pub outcome: Option<String>,
    pub reason_code: Option<String>,
    pub authorization_model_version: String,
    pub occurred_at: OffsetDateTime,
}

pub struct CapabilityMetricsRow {
    pub sessions_issued: u64,
    pub sessions_active: u64,
    pub sessions_expired: u64,
    pub sessions_revoked: u64,
    pub capability_calls: u64,
    pub ceiling_denials: u64,
    pub live_authorization_denials: u64,
    pub invalid_revisions: u64,
    pub average_revocation_latency_milliseconds: u64,
}

/// Redacted, authorized delivery evidence for one accepted mailbox event.
///
/// The application read model deliberately excludes the envelope, selected
/// headers, payload body, producer key, and trace context. Operators need the
/// scheduler's durable outcome, not the application-owned request content.
#[derive(FromRow)]
pub struct MailboxDeliveryInspectionRow {
    pub event_id: Uuid,
    pub mailbox_id: Uuid,
    pub disposition: String,
    pub logical_attempt_count: i32,
    pub denial_code: Option<String>,
    pub instance_revision_id: Option<Uuid>,
    pub state_volume_id: Option<Uuid>,
    pub lease_id: Option<Uuid>,
    pub lease_fencing_token: Option<i64>,
    pub dispatch_sequence: Option<i64>,
    pub state_access_outcome: Option<String>,
    pub next_eligible_at: Option<OffsetDateTime>,
    pub next_recovery_action: String,
    pub updated_at: OffsetDateTime,
}

pub struct InstanceSnapshot {
    pub instance: InstanceRow,
    pub revisions: Vec<RevisionRow>,
    pub attachments: Vec<AttachmentRow>,
    pub updates: Vec<UpdateRow>,
    pub repositories: Vec<RepositoryRow>,
    pub imports: Vec<ImportRow>,
    pub candidates: Vec<CandidateRow>,
    pub recent_runs: Vec<RecentRunRow>,
    pub capability_requirements: Vec<CapabilityRequirementRow>,
    pub capability_resources: Vec<CapabilityResourceOptionRow>,
    pub capability_bindings: Vec<CapabilityBindingRow>,
    pub runtime_sessions: Vec<RuntimeSessionRow>,
    pub capability_audit: Vec<CapabilityAuditRow>,
    pub capability_metrics: CapabilityMetricsRow,
    pub mailbox_deliveries: Vec<MailboxDeliveryInspectionRow>,
}
