use serde_json::Value;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct DeploymentRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub deployment_key: String,
    pub recipe_id: String,
    pub recipe_version: String,
    pub canonical_toml: String,
    pub declaration_hash: Vec<u8>,
    pub resolved_json: Vec<u8>,
    pub resolved_hash: Vec<u8>,
    pub input_hash: Vec<u8>,
    pub ordinary_inputs: Value,
    pub admission_evidence: Value,
    pub resource_count: i32,
    pub lifecycle: String,
    pub version: i64,
}

#[derive(sqlx::FromRow)]
pub struct ResourceRow {
    pub resource_name: String,
    pub resource_kind: String,
    pub resource_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub filesystem_uuid: Option<Uuid>,
    pub ownership: String,
    pub removal_policy: String,
    pub plan_json: Value,
    pub input_hash: Vec<u8>,
    pub install_progress: String,
    pub removal_progress: String,
    pub version: i64,
    pub active_attempt_id: Option<Uuid>,
    pub diagnostic: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct ReceiptRow {
    pub id: Uuid,
    pub deployment_id: Uuid,
    pub project_id: Uuid,
    pub input_hash: Vec<u8>,
    pub receipt_lifecycle: String,
    pub receipt_version: i64,
    pub event_id: Uuid,
    pub event_cursor: i64,
    pub event_aggregate_version: i64,
}
