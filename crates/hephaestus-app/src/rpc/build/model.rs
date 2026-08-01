use crate::application::build::{BuildActionError, BuildError, BuildMetric, BuildState, BuildView};
use rpc_proto::messages::hephaestus::{
    build::v1::{
        Build, BuildState as ProtoBuildState, BuildTimelineEntry, DeclaredArtifact,
        ProducedArtifact,
    },
    common::v1::{MetricLabel, OpaqueId, RuntimeMetric},
};
use time::OffsetDateTime;
use uuid::Uuid;

pub(in crate::rpc) fn build(value: BuildView) -> Build {
    Build {
        id: opaque(value.id).into(),
        repository_id: opaque(value.repository_id).into(),
        state: proto_state(value.state).into(),
        exit_code: value.exit_code,
        failure_code: value.failure_code.unwrap_or_default(),
        logs: value.logs,
        metrics: value.metrics.into_iter().map(metric).collect(),
        created_at: timestamp(value.created_at).into(),
        updated_at: timestamp(value.updated_at).into(),
        source_commit: value.source_commit,
        source_ref: value.source_ref,
        build_definition_hash: value.build_definition_hash,
        release_id: value.release_id.map(opaque).into(),
        release_state: value.release_state.unwrap_or_default(),
        artifact_count: value.artifact_count,
        trigger: value.trigger,
        agent_key: value.agent_key.unwrap_or_default(),
        builder_image_id: value.builder_image_id.map(opaque).into(),
        builder_image_key: value.builder_image_key.unwrap_or_default(),
        builder_image_reference: value.builder_image_reference.unwrap_or_default(),
        configuration_hash: value.configuration_hash.unwrap_or_default(),
        parsed_declaration_json: serde_json::to_string(&value.parsed_declaration)
            .unwrap_or_default(),
        build_policy_json: serde_json::to_string(&value.build_policy).unwrap_or_default(),
        started_at: value.started_at.map(timestamp).into(),
        completed_at: value.completed_at.map(timestamp).into(),
        duration_milliseconds: value.duration_milliseconds.unwrap_or_default(),
        timeline: value.timeline.into_iter().map(timeline).collect(),
        declared_artifacts: value
            .declared_artifacts
            .into_iter()
            .map(declared_artifact)
            .collect(),
        produced_artifacts: value
            .produced_artifacts
            .into_iter()
            .map(produced_artifact)
            .collect(),
        artifact_manifest_json: serde_json::to_string(&value.artifact_manifest).unwrap_or_default(),
        release_version: value.release_version.unwrap_or_default(),
        ..Default::default()
    }
}

fn timeline(value: crate::application::build::BuildTimelineEntry) -> BuildTimelineEntry {
    BuildTimelineEntry {
        from_state: value.from_state.unwrap_or_default(),
        to_state: value.to_state,
        reason: value.reason,
        occurred_at: timestamp(value.occurred_at).into(),
        ..Default::default()
    }
}

fn declared_artifact(value: crate::application::build::DeclaredArtifactView) -> DeclaredArtifact {
    DeclaredArtifact {
        path: value.path,
        kind: value.kind,
        media_type: value.media_type.unwrap_or_default(),
        ..Default::default()
    }
}

fn produced_artifact(value: crate::application::build::ProducedArtifactView) -> ProducedArtifact {
    ProducedArtifact {
        path: value.path,
        kind: value.kind,
        mode: value.mode,
        sha256: value.sha256,
        size_bytes: value.size_bytes,
        media_type: value.media_type,
        ..Default::default()
    }
}

pub(super) const fn operation_state(
    value: BuildState,
) -> rpc_proto::messages::hephaestus::common::v1::OperationState {
    use rpc_proto::messages::hephaestus::common::v1::OperationState;

    match value {
        BuildState::Queued => OperationState::Queued,
        BuildState::Running => OperationState::Running,
        BuildState::Succeeded => OperationState::Succeeded,
        BuildState::Failed => OperationState::Failed,
        BuildState::Cancelled => OperationState::Cancelled,
    }
}

const fn proto_state(value: BuildState) -> ProtoBuildState {
    match value {
        BuildState::Queued => ProtoBuildState::Queued,
        BuildState::Running => ProtoBuildState::Running,
        BuildState::Succeeded => ProtoBuildState::Succeeded,
        BuildState::Failed => ProtoBuildState::Failed,
        BuildState::Cancelled => ProtoBuildState::Cancelled,
    }
}

fn metric(value: BuildMetric) -> RuntimeMetric {
    RuntimeMetric {
        name: value.name,
        value: value.value,
        labels: value
            .labels
            .into_iter()
            .map(|(key, value)| MetricLabel {
                key,
                value,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
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

pub(super) fn application_error(error: BuildError) -> super::super::RpcError {
    use super::super::RpcError;

    match error {
        BuildError::NotFound => RpcError::NotFound,
        BuildError::FailedPrecondition => RpcError::FailedPrecondition,
        BuildError::InvalidStoredData | BuildError::Serialization(_) => {
            tracing::error!(%error, "stored build data could not be represented");
            RpcError::Internal
        }
        BuildError::Persistence(source) => {
            tracing::error!(error = %source, "build application persistence failed");
            RpcError::Unavailable
        }
    }
}

pub(super) fn action_error(error: BuildActionError) -> super::super::RpcError {
    match error {
        BuildActionError::Application(error) => application_error(error),
        BuildActionError::RetryNotAllowed
        | BuildActionError::RetryUnavailable
        | BuildActionError::VerificationNotAllowed
        | BuildActionError::VerificationUnavailable => super::super::RpcError::FailedPrecondition,
    }
}

#[cfg(test)]
mod tests {
    use super::action_error;
    use crate::application::build::BuildActionError;
    use crate::rpc::RpcError;

    #[test]
    fn unsupported_actions_map_to_a_lifecycle_precondition() {
        assert_eq!(
            action_error(BuildActionError::RetryUnavailable),
            RpcError::FailedPrecondition
        );
        assert_eq!(
            action_error(BuildActionError::VerificationUnavailable),
            RpcError::FailedPrecondition
        );
    }
}
