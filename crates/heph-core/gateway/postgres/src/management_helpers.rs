//! Validation and deterministic hashing helpers for gateway management.

use super::{GatewayMailboxBindingRow, GatewayManagementError, GatewaySecretSelection};
use gateway_domain::{GatewayServiceConfig, ServiceLogCaptureMode, ServiceProbePath};
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseCommandKey;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub fn valid_header_name(value: &str) -> bool {
    http::HeaderName::from_bytes(value.as_bytes()).is_ok()
}

pub fn configure_payload_hash(
    gateway_id: Uuid,
    expected_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    parameter_hash: &[u8],
    selections: &[GatewaySecretSelection],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.gateway.configure.v1\0");
    digest.update(gateway_id.as_bytes());
    digest.update(expected_revision_id.as_bytes());
    digest.update(release_id.as_bytes());
    digest.update(release_agent_id.as_bytes());
    digest.update(parameter_hash);
    let mut ordered = selections.to_vec();
    ordered.sort_by(|left, right| {
        left.slot_key
            .cmp(&right.slot_key)
            .then_with(|| left.route_path.cmp(&right.route_path))
            .then_with(|| left.header_name.cmp(&right.header_name))
    });
    for selection in ordered {
        digest.update(selection.slot_key.as_bytes());
        digest.update([0]);
        digest.update(selection.import_id.as_bytes());
        digest.update(selection.secret_version_id.as_bytes());
        digest.update(selection.route_path.as_bytes());
        digest.update([0]);
        digest.update(selection.header_name.as_bytes());
        digest.update([0]);
    }
    digest.finalize().into()
}

pub fn secret_selection_hash(selection: &GatewaySecretSelection, revision_id: Uuid) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.gateway.secret-selection.v1\0");
    digest.update(revision_id.as_bytes());
    digest.update(selection.slot_key.as_bytes());
    digest.update(selection.import_id.as_bytes());
    digest.update(selection.secret_version_id.as_bytes());
    digest.update(selection.route_path.as_bytes());
    digest.update(selection.header_name.as_bytes());
    digest.finalize().into()
}

pub fn valid_gateway_slot(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

pub fn valid_gateway_producer(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

pub fn service_config_from_columns(
    loopback_port: Option<i32>,
    readiness_path: Option<String>,
    health_path: Option<String>,
    log_capture_mode: &str,
) -> Result<Option<GatewayServiceConfig>, GatewayManagementError> {
    let log_capture_mode = ServiceLogCaptureMode::from_name(log_capture_mode)
        .ok_or(GatewayManagementError::Unavailable)?;
    match (loopback_port, readiness_path, health_path) {
        (None, None, None) if log_capture_mode.is_disabled() => Ok(None),
        (Some(port), Some(readiness), Some(health)) => {
            let port = u16::try_from(port).map_err(|_| GatewayManagementError::Unavailable)?;
            let readiness = ServiceProbePath::parse(readiness)
                .map_err(|_| GatewayManagementError::Unavailable)?;
            let health =
                ServiceProbePath::parse(health).map_err(|_| GatewayManagementError::Unavailable)?;
            GatewayServiceConfig::new(port, readiness, health)
                .map(|service| service.with_log_capture_mode(log_capture_mode))
                .map(Some)
                .map_err(|_| GatewayManagementError::Unavailable)
        }
        _ => Err(GatewayManagementError::Unavailable),
    }
}
pub async fn load_mailbox_binding(
    tx: &mut Transaction<'_, Postgres>,
    binding_id: Uuid,
) -> Result<Option<GatewayMailboxBindingRow>, sqlx::Error> {
    sqlx::query_as::<_, GatewayMailboxBindingRow>(
        "SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id,
                binding.slot_key, binding.producer_id, binding_grant.id AS grant_id,
                binding_grant.status AS grant_status, binding.created_at,
                binding_grant.granted_at, binding_grant.revoked_at
         FROM gateway_mailbox_bindings binding
         JOIN gateway_mailbox_binding_grants binding_grant
           ON binding_grant.binding_id = binding.id
         WHERE binding.id = $1",
    )
    .bind(binding_id)
    .fetch_optional(&mut **tx)
    .await
}

pub fn binding_command_key(identity: &AuthenticatedIdentity, operation: &str) -> ReleaseCommandKey {
    ReleaseCommandKey::derive(operation, &[identity.idempotency_id.as_uuid().as_bytes()])
}

pub fn binding_payload_hash(
    operation: &str,
    gateway_revision_id: Uuid,
    slot_key: &str,
    mailbox_id: Option<Uuid>,
    producer_id: &str,
    target_binding_id: Option<Uuid>,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(operation.as_bytes());
    digest.update([0]);
    digest.update(gateway_revision_id.as_bytes());
    digest.update([0]);
    digest.update(slot_key.as_bytes());
    digest.update([0]);
    if let Some(mailbox_id) = mailbox_id {
        digest.update(mailbox_id.as_bytes());
    }
    digest.update([0]);
    digest.update(producer_id.as_bytes());
    digest.update([0]);
    if let Some(target_binding_id) = target_binding_id {
        digest.update(target_binding_id.as_bytes());
    }
    digest.finalize().into()
}
