use build_orchestrator::{BuildInput, BuildRepositoryError, ClaimedBuild};
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseId, ReleaseVersion};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

pub fn storage(error: impl std::error::Error + Send + Sync + 'static) -> BuildRepositoryError {
    BuildRepositoryError::Storage(Box::new(error))
}

#[derive(Debug, FromRow)]
pub struct InputRow {
    pub repository_id: Uuid,
    pub source_commit: String,
    pub source_ref: String,
    pub image_reference: Option<String>,
    pub state: String,
    pub config: Value,
    pub created_by: Option<Uuid>,
}

pub fn claimed(
    id: BuildRequestId,
    row: InputRow,
    release_id: ReleaseId,
    agent: ReleaseAgentId,
    version: ReleaseVersion,
) -> Result<ClaimedBuild, BuildRepositoryError> {
    let config: agent_config::AgentConfig =
        serde_json::from_value(row.config).map_err(|_| BuildRepositoryError::InvalidData)?;
    let build = config.build.ok_or(BuildRepositoryError::InvalidData)?;
    let image_reference = row
        .image_reference
        .ok_or(BuildRepositoryError::InvalidData)?;
    Ok(ClaimedBuild {
        input: BuildInput {
            id,
            repository_id: row.repository_id,
            source_commit: row.source_commit,
            source_ref: row.source_ref,
            build,
            image_reference,
        },
        release_id,
        release_agent_id: agent,
        release_version: version,
    })
}

pub fn manifest_projection(value: &Value) -> Vec<Value> {
    let mut values = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let object = entry.as_object()?;
            Some(serde_json::json!({
                "path": object.get("path")?,
                "kind": object.get("kind")?,
                "mode": object.get("mode")?,
                "content_hash": object.get("content_hash")?,
                "size_bytes": object.get("size_bytes")?,
                "media_type": object.get("media_type")?,
            }))
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        left.get("path")
            .and_then(Value::as_str)
            .cmp(&right.get("path").and_then(Value::as_str))
    });
    values
}
