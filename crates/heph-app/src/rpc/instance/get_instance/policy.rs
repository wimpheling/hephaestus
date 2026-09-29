use crate::rpc::RpcError;
use buffa_types::google::protobuf::Timestamp;
use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
use rpc_proto::messages::hephaestus::{
    common::v1::OperationState,
    secret::v1::{
        AuthorityState, DeliveryMode, DeliveryPhase, SecretState, SecretTarget, secret_target,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(super) fn timestamp(value: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}

pub(super) fn operation_state(value: &str) -> OperationState {
    match value {
        "queued" => OperationState::Queued,
        "running" => OperationState::Running,
        "succeeded" => OperationState::Succeeded,
        "failed" => OperationState::Failed,
        "cancelled" => OperationState::Cancelled,
        _ => OperationState::Unspecified,
    }
}

pub(super) fn target(kind: &str, id: Uuid) -> Result<SecretTarget, RpcError> {
    let target = match kind {
        "project" => secret_target::Target::ProjectId(Box::new(opaque(id))),
        "repository" => secret_target::Target::RepositoryId(Box::new(opaque(id))),
        _ => return Err(RpcError::Internal),
    };
    Ok(SecretTarget {
        target: Some(target),
        ..Default::default()
    })
}
pub(super) fn authority(value: &str) -> Result<AuthorityState, RpcError> {
    match value {
        "active" => Ok(AuthorityState::Active),
        "revoked" => Ok(AuthorityState::Revoked),
        "expired" => Ok(AuthorityState::Expired),
        _ => Err(RpcError::Internal),
    }
}
pub(super) fn secret_state(value: &str) -> Result<SecretState, RpcError> {
    match value {
        "active" => Ok(SecretState::Active),
        "disabled" => Ok(SecretState::Disabled),
        "revoked" => Ok(SecretState::Revoked),
        "purged" => Ok(SecretState::Purged),
        _ => Err(RpcError::Internal),
    }
}
pub(super) fn delivery(value: &str) -> Result<buffa::EnumValue<DeliveryMode>, RpcError> {
    match value {
        "raw" => Ok(DeliveryMode::Raw.into()),
        "brokered" => Ok(DeliveryMode::Brokered.into()),
        _ => Err(RpcError::Internal),
    }
}
pub(super) fn phase(value: &str) -> Result<buffa::EnumValue<DeliveryPhase>, RpcError> {
    match value {
        "normal" => Ok(DeliveryPhase::Normal.into()),
        "update" => Ok(DeliveryPhase::Update.into()),
        _ => Err(RpcError::Internal),
    }
}
pub(super) fn encode_hex(value: &[u8]) -> String {
    value.iter().fold(
        String::with_capacity(value.len() * 2),
        |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        },
    )
}
