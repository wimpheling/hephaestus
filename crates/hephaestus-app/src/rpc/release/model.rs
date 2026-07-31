use crate::application::{
    build::BuildView,
    release::{
        ReleaseAgent as ApplicationAgent, ReleaseArtifact as ApplicationArtifact, ReleaseError,
        ReleaseState as ApplicationState, ReleaseSummary as ApplicationSummary,
    },
};
use agent_config::{SecretDeliveryMode, SecretPhase, SecretSlotDeclaration};
use release_domain::{
    NetworkAccess, ParameterDeclaration as ApplicationParameter, ParameterType, ParameterValue,
};
use rpc_proto::messages::hephaestus::{
    artifact::v1::{Artifact, ArtifactProvenance},
    common::v1::{
        BooleanParameterConstraints, EnumParameterConstraints, IntegerParameterConstraints,
        NetworkPolicy, OpaqueId, ParameterDeclaration, ParameterDefault,
        ParameterType as ProtoParameterType, RuntimeContract, RuntimePolicy,
        SecretSlotDeclaration as ProtoSecretSlot, SecretSlotDeliveryMode, SecretSlotPhase,
        StringParameterConstraints, UpdateHook, parameter_default,
    },
    release::v1::{ReleaseAgent, ReleaseState, ReleaseSummary},
};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) fn summary(value: ApplicationSummary) -> ReleaseSummary {
    ReleaseSummary {
        id: opaque(value.id).into(),
        version: value.version,
        state: state(value.state).into(),
        source_commit: value.source_commit,
        source_ref: value.source_ref,
        build_request_id: opaque(value.build_request_id).into(),
        created_at: timestamp(value.created_at).into(),
        published_at: value.published_at.map(timestamp).into(),
        manifest_hash: value.manifest_hash,
        artifact_count: value.artifact_count,
        exported_agent_count: value.agent_count,
        ..Default::default()
    }
}

pub(super) fn artifact(
    value: ApplicationArtifact,
    release_id: Uuid,
    build_id: Uuid,
    source_commit: &str,
) -> Artifact {
    Artifact {
        id: opaque(value.id).into(),
        path: value.path,
        kind: value.kind,
        mode: value.mode,
        sha256: value.sha256,
        size_bytes: value.size_bytes,
        media_type: value.media_type,
        provenance: ArtifactProvenance {
            build_id: opaque(build_id).into(),
            release_id: opaque(release_id).into(),
            source_commit: source_commit.to_owned(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

pub(super) fn agent(value: ApplicationAgent) -> ReleaseAgent {
    let update_hook = value.update_hook.as_ref().map(|hook| UpdateHook {
        required: true,
        timeout_seconds: hook.timeout_seconds,
        ..Default::default()
    });
    ReleaseAgent {
        id: opaque(value.id).into(),
        family_id: opaque(value.family_id).into(),
        agent_key: value.agent_key,
        display_name: value.display_name,
        runtime_contract: RuntimeContract {
            policy_ceiling: runtime_policy(&value.policy).into(),
            requires_state: value.requires_state,
            ..Default::default()
        }
        .into(),
        parameter_schema: value.parameter_schema.into_iter().map(parameter).collect(),
        secret_slot_schema: value.secret_slots.into_iter().map(secret_slot).collect(),
        requires_state: value.requires_state,
        update_hook: update_hook.into(),
        created_at: timestamp(value.created_at).into(),
        ..Default::default()
    }
}

fn runtime_policy(value: &release_domain::RuntimePolicy) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: u32::from(value.vcpus),
        memory_mib: value.memory_mib,
        network: match value.network {
            NetworkAccess::Disabled => NetworkPolicy::Disabled,
            NetworkAccess::BrokerOnly => NetworkPolicy::BrokerOnly,
            NetworkAccess::Egress => NetworkPolicy::Egress,
        }
        .into(),
        ..Default::default()
    }
}

fn parameter(value: ApplicationParameter) -> ParameterDeclaration {
    let name = value.name.to_string();
    let constraint = match value.value_type {
        ParameterType::String {
            minimum_length,
            maximum_length,
        } => StringParameterConstraints {
            minimum_length: u32::from(minimum_length),
            maximum_length: u32::from(maximum_length),
            ..Default::default()
        }
        .into(),
        ParameterType::Integer { minimum, maximum } => IntegerParameterConstraints {
            minimum,
            maximum,
            ..Default::default()
        }
        .into(),
        ParameterType::Boolean => BooleanParameterConstraints::default().into(),
        ParameterType::Enum { values } => EnumParameterConstraints {
            values,
            ..Default::default()
        }
        .into(),
    };
    let default = value.default.map(|value| ParameterDefault {
        value: Some(match value {
            ParameterValue::String(value) => parameter_default::Value::StringValue(value),
            ParameterValue::Integer(value) => parameter_default::Value::IntegerValue(value),
            ParameterValue::Boolean(value) => parameter_default::Value::BooleanValue(value),
        }),
        ..Default::default()
    });
    ParameterDeclaration {
        label: name.clone(),
        name,
        value_type: ProtoParameterType {
            constraint: Some(constraint),
            ..Default::default()
        }
        .into(),
        required: value.required,
        default: default.into(),
        sensitive: value.sensitive,
        ..Default::default()
    }
}

fn secret_slot(value: SecretSlotDeclaration) -> ProtoSecretSlot {
    ProtoSecretSlot {
        key: value.key,
        purpose: value.purpose,
        required: value.required,
        delivery_modes: value
            .delivery_modes
            .into_iter()
            .map(|mode| {
                match mode {
                    SecretDeliveryMode::Raw => SecretSlotDeliveryMode::Raw,
                    SecretDeliveryMode::Brokered => SecretSlotDeliveryMode::Brokered,
                }
                .into()
            })
            .collect(),
        phases: value
            .phases
            .into_iter()
            .map(|phase| {
                match phase {
                    SecretPhase::Normal => SecretSlotPhase::Normal,
                    SecretPhase::Update => SecretSlotPhase::Update,
                }
                .into()
            })
            .collect(),
        destinations: value.destinations,
        ..Default::default()
    }
}

pub(super) fn build(value: BuildView) -> rpc_proto::messages::hephaestus::build::v1::Build {
    super::super::build::model::build(value)
}

pub(super) const fn state(value: ApplicationState) -> ReleaseState {
    match value {
        ApplicationState::Draft => ReleaseState::Draft,
        ApplicationState::Published => ReleaseState::Published,
        ApplicationState::Revoked => ReleaseState::Revoked,
    }
}

pub(super) fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

pub(super) fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}

pub(super) fn application_error(error: ReleaseError) -> super::super::RpcError {
    use super::super::RpcError;

    match error {
        ReleaseError::NotFound => RpcError::NotFound,
        ReleaseError::InvalidPage => RpcError::InvalidArgument,
        ReleaseError::InvalidStoredData | ReleaseError::Serialization(_) => {
            tracing::error!(%error, "stored release data could not be represented");
            RpcError::Internal
        }
        ReleaseError::Persistence(source) => {
            tracing::error!(error = %source, "release application persistence failed");
            RpcError::Unavailable
        }
    }
}
