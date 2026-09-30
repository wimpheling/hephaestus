//! Lease validation, append row models, and storage helpers.

use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceLogStoreError, GatewayServiceOwner,
    MAX_SERVICE_OWNER_HOST_BYTES,
};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

pub const fn stream_name(stream: LogStream) -> &'static str {
    match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
        _ => "unknown",
    }
}

pub fn validate_lease(
    lease: &GatewayServiceInstanceLease,
) -> Result<(), GatewayServiceLogStoreError> {
    if lease.identity.instance_id.is_nil()
        || lease.identity.gateway_id.is_nil()
        || lease.identity.revision_id.is_nil()
        || lease.owner_uuid.is_nil()
        || lease.fencing_token <= 0
        || lease.vm_id != format!("gateway-service-{}", lease.identity.instance_id)
        || lease.owner_host_id.is_empty()
        || lease.owner_host_id.len() > MAX_SERVICE_OWNER_HOST_BYTES
        || lease.owner_host_id != lease.owner_host_id.trim()
        || lease
            .owner_host_id
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(GatewayServiceLogStoreError::InvalidArgument);
    }
    Ok(())
}

pub fn ensure_current_lease(
    current: &InstanceRow,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    now: OffsetDateTime,
) -> Result<(), GatewayServiceLogStoreError> {
    if current.id != lease.identity.instance_id
        || current.owner_host_id != lease.owner_host_id
        || current.owner_uuid != lease.owner_uuid
        || current.owner_host_id != owner.host_id
        || current.owner_uuid != owner.owner_uuid
        || current.fencing_token != lease.fencing_token
        || current.vm_id != lease.vm_id
        || current.state == "cleaned"
        || current.lease_expires_at <= now
    {
        return Err(GatewayServiceLogStoreError::StaleLease);
    }
    Ok(())
}

pub fn storage(_error: sqlx::Error) -> GatewayServiceLogStoreError {
    GatewayServiceLogStoreError::Unavailable
}

#[derive(Debug, FromRow)]
pub struct UsageRow {
    pub bytes: i64,
    pub chunks: i64,
    pub epochs: i32,
}

#[derive(Debug, FromRow)]
pub struct EpochRow {
    pub acknowledged_through: i64,
    pub retained_bytes: i64,
    pub retained_chunks: i64,
    pub producer_dropped_chunks: i64,
    pub producer_dropped_bytes: i64,
    pub provider_lagged_events: i64,
    pub storage_dropped_chunks: i64,
    pub storage_dropped_bytes: i64,
}

#[derive(Debug, FromRow)]
pub struct InstanceUsageRow {
    pub bytes: i64,
    pub chunks: i64,
}

#[derive(FromRow)]
pub struct ChunkRow {
    pub stream: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, FromRow)]
pub struct InstanceRow {
    pub id: Uuid,
    pub gateway_id: Uuid,
    pub revision_id: Uuid,
    pub owner_host_id: String,
    pub owner_uuid: Uuid,
    pub fencing_token: i64,
    pub vm_id: String,
    pub state: String,
    pub lease_expires_at: OffsetDateTime,
}
