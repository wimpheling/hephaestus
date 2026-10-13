use serde_json::Value;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct OriginalRow {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub idempotency_id: Uuid,
    pub input_hash: Vec<u8>,
    pub first_request_id: Uuid,
    pub receipt_lifecycle: String,
    pub receipt_version: i64,
    pub created_by: Uuid,
    pub deployment_request_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub struct HeaderRow {
    pub original_actor_id: Uuid,
    pub project_id: Uuid,
    pub command_id: Uuid,
    pub actor_id: Uuid,
    pub idempotency_id: Uuid,
    pub operation: String,
    pub version: i32,
    pub profile: String,
    pub intent_hash: Vec<u8>,
    pub platform_json: Value,
    pub instance_count: i32,
    pub canonical_input: Vec<u8>,
    pub input_hash: Vec<u8>,
    pub request_id: Uuid,
    pub event_id: Uuid,
    pub event_cursor: i64,
    pub event_aggregate_version: i64,
    pub event_valid: bool,
}

#[derive(sqlx::FromRow)]
pub struct ImportRow {
    pub resource_name: String,
    pub ordinal: i32,
    pub operation_id: Uuid,
    pub import_command_key: Vec<u8>,
    pub instance_id: Uuid,
    pub revision_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub canonical_import: Vec<u8>,
    pub input_hash: Vec<u8>,
}

#[derive(sqlx::FromRow)]
pub struct WriterRow {
    pub event_id: Uuid,
    pub event_cursor: i64,
    pub event_aggregate_version: i64,
    pub replayed: bool,
}
