use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(FromRow)]
pub struct ActiveRow {
    pub id: Uuid,
    pub command_id: Uuid,
    pub deployment_id: Uuid,
    pub project_id: Uuid,
    pub resource_name: String,
    pub action: String,
    pub generation: i64,
    pub input_hash: Vec<u8>,
    pub claim_json: sqlx::types::Json<Value>,
    pub before_state: sqlx::types::Json<Value>,
    pub request_fingerprint: Vec<u8>,
    pub expected_deployment_version: i64,
    pub actor_id: Uuid,
    pub request_id: Uuid,
    pub command_actor: Uuid,
    pub idempotency_id: Uuid,
    pub operation: String,
    pub claimed_state: Option<sqlx::types::Json<Value>>,
    pub proof_valid: bool,
}
