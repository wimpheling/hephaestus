use super::{
    BuildError, BuildMetric, BuildState, BuildTimelineEntry, BuildVerificationView, BuildView,
    DeclaredArtifactView, ProducedArtifactView,
};
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;
use std::collections::BTreeMap;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(FromRow)]
pub(super) struct BuildRow {
    id: Uuid,
    repository_id: Uuid,
    state: String,
    exit_code: Option<i32>,
    failure_code: Option<String>,
    logs: Option<Value>,
    metrics: Option<Value>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    source_commit: String,
    source_ref: String,
    build_definition_hash: String,
    build_trigger: String,
    agent_key: Option<String>,
    image_id: Option<Uuid>,
    image_key: Option<String>,
    image_reference: Option<String>,
    configuration_hash: Option<String>,
    build_declaration: Option<Value>,
    build_policy: Option<Value>,
    declared_artifacts: Option<Value>,
    started_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
    artifact_manifest: Option<Value>,
    release_id: Option<Uuid>,
    release_state: Option<String>,
    release_version: Option<String>,
    artifact_count: i64,
    timeline: Option<Value>,
    produced_artifacts: Option<Value>,
    verifications: Option<Value>,
}

#[derive(FromRow)]
pub(super) struct BuildSourceRow {
    pub(super) receive_id: Uuid,
    pub(super) config: Value,
    pub(super) git_ref: String,
}

#[derive(Deserialize)]
struct StoredLog {
    stream: String,
    text: String,
}

#[derive(Deserialize)]
struct StoredMetric {
    name: String,
    value: f64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct StoredTimelineEntry {
    from_state: Option<String>,
    to_state: String,
    reason: String,
    occurred_at: String,
}

impl TryFrom<StoredTimelineEntry> for BuildTimelineEntry {
    type Error = BuildError;

    fn try_from(value: StoredTimelineEntry) -> Result<Self, Self::Error> {
        if value.to_state.is_empty() || value.reason.is_empty() {
            return Err(BuildError::InvalidStoredData);
        }
        let occurred_at = OffsetDateTime::parse(&value.occurred_at, &Rfc3339)
            .map_err(|_| BuildError::InvalidStoredData)?;
        Ok(Self {
            from_state: value.from_state,
            to_state: value.to_state,
            reason: value.reason,
            occurred_at,
        })
    }
}

#[derive(Deserialize)]
struct StoredDeclaredArtifact {
    path: String,
    kind: String,
    media_type: Option<String>,
}

impl From<StoredDeclaredArtifact> for DeclaredArtifactView {
    fn from(value: StoredDeclaredArtifact) -> Self {
        Self {
            path: value.path,
            kind: value.kind,
            media_type: value.media_type,
        }
    }
}

#[derive(Deserialize)]
struct StoredProducedArtifact {
    path: String,
    kind: String,
    mode: i32,
    sha256: String,
    size_bytes: i64,
    media_type: String,
}

#[derive(Deserialize)]
struct StoredVerification {
    state: String,
    expected_manifest: Value,
    actual_manifest: Option<Value>,
    failure_code: Option<String>,
    created_at: String,
    completed_at: Option<String>,
}

impl TryFrom<StoredVerification> for BuildVerificationView {
    type Error = BuildError;

    fn try_from(value: StoredVerification) -> Result<Self, Self::Error> {
        let created_at = OffsetDateTime::parse(&value.created_at, &Rfc3339)
            .map_err(|_| BuildError::InvalidStoredData)?;
        let completed_at = value
            .completed_at
            .as_deref()
            .map(|timestamp| OffsetDateTime::parse(timestamp, &Rfc3339))
            .transpose()
            .map_err(|_| BuildError::InvalidStoredData)?;
        if !matches!(value.state.as_str(), "running" | "succeeded" | "failed") {
            return Err(BuildError::InvalidStoredData);
        }
        if value.state == "failed" && value.failure_code.is_none() {
            return Err(BuildError::InvalidStoredData);
        }
        Ok(Self {
            state: value.state,
            expected_manifest: value.expected_manifest,
            actual_manifest: value.actual_manifest,
            failure_code: value.failure_code,
            created_at,
            completed_at,
        })
    }
}

impl TryFrom<StoredProducedArtifact> for ProducedArtifactView {
    type Error = BuildError;

    fn try_from(value: StoredProducedArtifact) -> Result<Self, Self::Error> {
        Ok(Self {
            path: value.path,
            kind: value.kind,
            mode: u32::try_from(value.mode).map_err(|_| BuildError::InvalidStoredData)?,
            sha256: value.sha256,
            size_bytes: u64::try_from(value.size_bytes)
                .map_err(|_| BuildError::InvalidStoredData)?,
            media_type: value.media_type,
        })
    }
}

impl TryFrom<BuildRow> for BuildView {
    type Error = BuildError;

    fn try_from(row: BuildRow) -> Result<Self, Self::Error> {
        let state = parse_state(&row.state)?;
        let logs = serde_json::from_value::<Vec<StoredLog>>(
            row.logs.unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(|entry| format!("[{}] {}", entry.stream, entry.text))
        .collect();
        let metrics = serde_json::from_value::<Vec<StoredMetric>>(
            row.metrics.unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(|metric| BuildMetric {
            name: metric.name,
            value: metric.value,
            labels: metric.labels,
        })
        .collect();
        let timeline = serde_json::from_value::<Vec<StoredTimelineEntry>>(
            row.timeline.unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(BuildTimelineEntry::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let declared_artifacts = serde_json::from_value::<Vec<StoredDeclaredArtifact>>(
            row.declared_artifacts
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(DeclaredArtifactView::from)
        .collect();
        let produced_artifacts = serde_json::from_value::<Vec<StoredProducedArtifact>>(
            row.produced_artifacts
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(ProducedArtifactView::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let verifications = serde_json::from_value::<Vec<StoredVerification>>(
            row.verifications
                .unwrap_or_else(|| Value::Array(Vec::new())),
        )
        .map_err(BuildError::Serialization)?
        .into_iter()
        .map(BuildVerificationView::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        let duration_milliseconds = row.started_at.map(|started| {
            let milliseconds = row
                .completed_at
                .unwrap_or_else(OffsetDateTime::now_utc)
                .unix_timestamp_nanos()
                .saturating_sub(started.unix_timestamp_nanos())
                / 1_000_000;
            i64::try_from(milliseconds.max(0)).unwrap_or(i64::MAX)
        });
        Ok(Self {
            id: row.id,
            repository_id: row.repository_id,
            state,
            exit_code: row.exit_code,
            failure_code: row.failure_code,
            logs,
            metrics,
            created_at: row.created_at,
            updated_at: row.updated_at,
            source_commit: row.source_commit,
            source_ref: row.source_ref,
            build_definition_hash: row.build_definition_hash,
            release_id: row.release_id,
            release_state: row.release_state,
            release_version: row.release_version,
            artifact_count: u32::try_from(row.artifact_count)
                .map_err(|_| BuildError::InvalidStoredData)?,
            trigger: row.build_trigger,
            agent_key: row.agent_key,
            image_id: row.image_id,
            image_key: row.image_key,
            image_reference: row.image_reference,
            configuration_hash: row.configuration_hash,
            parsed_declaration: row
                .build_declaration
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            build_policy: row
                .build_policy
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            started_at: row.started_at,
            completed_at: row.completed_at,
            duration_milliseconds,
            timeline,
            declared_artifacts,
            produced_artifacts,
            artifact_manifest: row
                .artifact_manifest
                .unwrap_or_else(|| Value::Array(Vec::new())),
            verifications,
        })
    }
}

pub(super) fn parse_state(value: &str) -> Result<BuildState, BuildError> {
    match value {
        "queued" => Ok(BuildState::Queued),
        "running" | "importing" => Ok(BuildState::Running),
        "succeeded" => Ok(BuildState::Succeeded),
        "failed" => Ok(BuildState::Failed),
        "cancelled" => Ok(BuildState::Cancelled),
        _ => Err(BuildError::InvalidStoredData),
    }
}
