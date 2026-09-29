//! Run aggregate to transport conversion helpers.

use super::errors::invalid;
use crate::application::run::{EventPayload, RunEvent as AppEvent, RunView};
use rpc_proto::messages::hephaestus::{
    artifact::v1::{Artifact, ArtifactProvenance},
    common::v1::{MetricLabel, OpaqueId, RuntimeMetric},
    run::v1::{ResultProposal, Run, RunEvent, RunFailure, RunMetrics, RunResult, run_event},
};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

// The protobuf snapshot intentionally maps the complete run aggregate together.
#[allow(clippy::too_many_lines)]
pub(super) fn proto_run(value: RunView) -> Result<Run, connectrpc::ConnectError> {
    let event_count = u64::try_from(value.events.len()).map_err(invalid)?;
    let log_count = u64::try_from(
        value
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::Log(_)))
            .count(),
    )
    .map_err(invalid)?;
    let runtime_metrics = value
        .events
        .iter()
        .filter_map(|event| {
            if let EventPayload::Metric {
                name,
                value,
                labels,
            } = &event.payload
            {
                Some(metric(name.clone(), *value, labels.clone()))
            } else {
                None
            }
        })
        .collect();
    let events = value
        .events
        .into_iter()
        .map(proto_event)
        .collect::<Result<Vec<_>, _>>()?;
    let artifacts = value
        .artifacts
        .into_iter()
        .map(|artifact| Artifact {
            id: opaque(artifact.id).into(),
            path: artifact.path,
            kind: artifact.kind,
            mode: artifact
                .mode
                .and_then(|mode| u32::try_from(mode).ok())
                .unwrap_or_default(),
            sha256: artifact.sha256,
            size_bytes: u64::try_from(artifact.size_bytes).unwrap_or_default(),
            media_type: artifact.media_type.unwrap_or_default(),
            provenance: ArtifactProvenance {
                run_id: opaque(value.id).into(),
                release_id: opaque(value.release_id).into(),
                source_commit: value.input_commit.clone(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .collect();
    let result = value.result_id.map(|id| RunResult {
        id: opaque(id).into(),
        commit: value.result_commit.unwrap_or_default(),
        r#ref: value.result_ref.unwrap_or_default(),
        tree: value.result_tree.unwrap_or_default(),
        message: value.result_message.unwrap_or_default(),
        artifact_manifest_hash: value.artifact_manifest_hash.unwrap_or_default(),
        proposal: value
            .proposal_id
            .map(|id| ResultProposal {
                id: opaque(id).into(),
                state: value.proposal_state.unwrap_or_default(),
                target_ref: value.proposal_target_ref.unwrap_or_default(),
                version: value
                    .proposal_version
                    .and_then(|version| u64::try_from(version).ok())
                    .unwrap_or_default(),
                ..Default::default()
            })
            .into(),
        ..Default::default()
    });
    let elapsed = (value.updated_at - value.created_at)
        .whole_milliseconds()
        .max(0);
    Ok(Run {
        id: opaque(value.id).into(),
        state: value.state,
        outcome: value.outcome.unwrap_or_default(),
        exit_code: value.exit_code,
        exit_signal: value.exit_signal,
        failure: value
            .failure
            .map(|code| RunFailure {
                code,
                ..Default::default()
            })
            .into(),
        created_at: timestamp(value.created_at).into(),
        updated_at: timestamp(value.updated_at).into(),
        state_version: u64::try_from(value.state_version).unwrap_or_default(),
        agent_id: opaque(value.agent_id).into(),
        agent_name: value.agent_name,
        instance_project_id: opaque(value.instance_project_id).into(),
        instance_project_name: value.instance_project_name,
        instance_revision_id: opaque(value.instance_revision_id).into(),
        release_id: opaque(value.release_id).into(),
        release_version: value.release_version,
        source_repository_id: opaque(value.source_repository_id).into(),
        repository_id: opaque(value.repository_id).into(),
        repository_name: value.repository_name,
        project_id: opaque(value.project_id).into(),
        project_name: value.project_name,
        organization_id: opaque(value.organization_id).into(),
        organization_name: value.organization_name,
        input_commit: value.input_commit,
        git_ref: value.git_ref,
        attempt: u32::try_from(value.attempt).unwrap_or_default(),
        retry_supported: value.retry_supported,
        result: result.into(),
        events,
        artifacts,
        metrics: RunMetrics {
            event_count,
            log_count,
            elapsed_ms: u64::try_from(elapsed).unwrap_or_default(),
            runtime_metrics,
            ..Default::default()
        }
        .into(),
        patch_preview: value.patch_preview,
        manifest_preview: value.manifest_preview,
        ..Default::default()
    })
}

pub(super) fn metric(
    name: String,
    value: f64,
    labels: std::collections::BTreeMap<String, String>,
) -> RuntimeMetric {
    RuntimeMetric {
        name,
        value,
        labels: labels
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
pub(super) fn proto_event(event: AppEvent) -> Result<RunEvent, connectrpc::ConnectError> {
    let payload = match event.payload {
        EventPayload::Log(message) => run_event::Payload::BoundedLogMessage(message),
        EventPayload::State(state) => run_event::Payload::State(state),
        EventPayload::Metric {
            name,
            value,
            labels,
        } => run_event::Payload::Metric(Box::new(metric(name, value, labels))),
    };
    Ok(RunEvent {
        sequence: u64::try_from(event.sequence).map_err(invalid)?,
        event_type: event.event_type,
        payload: Some(payload),
        occurred_at: timestamp(event.occurred_at).into(),
        ..Default::default()
    })
}

pub(super) fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}
