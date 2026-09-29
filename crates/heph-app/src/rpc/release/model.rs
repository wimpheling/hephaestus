use crate::application::release::ReleaseDetail;
use crate::application::release::{
    ReleaseError, ReleaseState as ApplicationState, ReleaseSummary as ApplicationSummary,
};
use crate::rpc::RpcError;
use rpc_proto::messages::hephaestus::{
    common::v1::OpaqueId,
    release::v1::{Release, ReleaseState, ReleaseSummary},
};
use time::OffsetDateTime;
use uuid::Uuid;

mod runtime;
#[cfg(test)]
#[path = "model/tests.rs"]
mod tests;
mod ui;

pub(super) use runtime::{agent, artifact, build};
pub(super) use ui::ui_descriptors;

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

pub(super) fn release(value: ReleaseDetail) -> Release {
    let release_id = value.summary.id;
    let build_id = value.summary.build_request_id;
    let source_commit = value.summary.source_commit.clone();
    Release {
        id: opaque(release_id).into(),
        version: value.summary.version,
        state: state(value.summary.state).into(),
        source_commit: value.summary.source_commit,
        source_ref: value.summary.source_ref,
        build_request_id: opaque(build_id).into(),
        build_definition_hash: value.build_definition_hash,
        configuration_hash: value.configuration_hash,
        manifest_hash: value.summary.manifest_hash,
        created_at: timestamp(value.summary.created_at).into(),
        published_at: value.summary.published_at.map(timestamp).into(),
        revoked_at: value.revoked_at.map(timestamp).into(),
        repository_id: opaque(value.repository_id).into(),
        repository_name: value.repository_name,
        project_id: opaque(value.project_id).into(),
        project_name: value.project_name,
        organization_id: opaque(value.organization_id).into(),
        organization_name: value.organization_name,
        build: build(value.build).into(),
        artifacts: value
            .artifacts
            .into_iter()
            .map(|artifact_value| artifact(artifact_value, release_id, build_id, &source_commit))
            .collect(),
        agents: value.agents.into_iter().map(agent).collect(),
        ui_descriptors: ui_descriptors(value.ui_descriptors),
        ..Default::default()
    }
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

pub(super) fn application_error(error: ReleaseError) -> RpcError {
    match error {
        ReleaseError::NotFound => RpcError::NotFound,
        ReleaseError::InvalidPage | ReleaseError::InvalidVersion => RpcError::InvalidArgument,
        ReleaseError::Conflict => RpcError::AlreadyExists,
        ReleaseError::FailedPrecondition => RpcError::FailedPrecondition,
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
