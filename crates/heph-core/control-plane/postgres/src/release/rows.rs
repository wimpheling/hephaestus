use super::types::{ReleaseAgent, ReleaseArtifact, ReleaseError, ReleaseState, ReleaseSummary};
use agent_config::SecretSlotDeclaration;
use release_domain::{RuntimePolicy, UpdateHook};
use serde::Deserialize;
use serde_json::Value;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(FromRow)]
pub struct ReleaseSummaryRow {
    id: Uuid,
    version: String,
    state: String,
    source_commit: String,
    source_ref: String,
    build_request_id: Uuid,
    created_at: OffsetDateTime,
    published_at: Option<OffsetDateTime>,
    manifest_hash: String,
    artifact_count: i64,
    agent_count: i64,
}

impl TryFrom<ReleaseSummaryRow> for ReleaseSummary {
    type Error = ReleaseError;

    fn try_from(row: ReleaseSummaryRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            version: row.version,
            state: parse_state(&row.state)?,
            source_commit: row.source_commit,
            source_ref: row.source_ref,
            build_request_id: row.build_request_id,
            created_at: row.created_at,
            published_at: row.published_at,
            manifest_hash: row.manifest_hash,
            artifact_count: u32::try_from(row.artifact_count)
                .map_err(|_| ReleaseError::InvalidStoredData)?,
            agent_count: u32::try_from(row.agent_count)
                .map_err(|_| ReleaseError::InvalidStoredData)?,
        })
    }
}

#[derive(FromRow)]
pub struct ReleaseDetailRow {
    pub id: Uuid,
    pub version: String,
    pub state: String,
    pub source_commit: String,
    pub source_ref: String,
    pub build_request_id: Uuid,
    pub created_at: OffsetDateTime,
    pub published_at: Option<OffsetDateTime>,
    pub manifest_hash: String,
    pub build_definition_hash: String,
    pub configuration_hash: String,
    pub revoked_at: Option<OffsetDateTime>,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
}

impl ReleaseDetailRow {
    pub fn summary(&self) -> Result<ReleaseSummary, ReleaseError> {
        ReleaseSummary::try_from(ReleaseSummaryRow {
            id: self.id,
            version: self.version.clone(),
            state: self.state.clone(),
            source_commit: self.source_commit.clone(),
            source_ref: self.source_ref.clone(),
            build_request_id: self.build_request_id,
            created_at: self.created_at,
            published_at: self.published_at,
            manifest_hash: self.manifest_hash.clone(),
            artifact_count: 0,
            agent_count: 0,
        })
    }
}

#[derive(FromRow)]
pub struct ArtifactRow {
    id: Uuid,
    path: String,
    kind: String,
    mode: i32,
    sha256: String,
    size_bytes: i64,
    media_type: String,
}

impl TryFrom<ArtifactRow> for ReleaseArtifact {
    type Error = ReleaseError;

    fn try_from(row: ArtifactRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            path: row.path,
            kind: row.kind,
            mode: u32::try_from(row.mode).map_err(|_| ReleaseError::InvalidStoredData)?,
            sha256: row.sha256,
            size_bytes: u64::try_from(row.size_bytes)
                .map_err(|_| ReleaseError::InvalidStoredData)?,
            media_type: row.media_type,
        })
    }
}

#[derive(FromRow)]
pub struct AgentRow {
    id: Uuid,
    family_id: Uuid,
    agent_key: String,
    display_name: String,
    runtime_contract: Value,
    parameter_schema: Value,
    secret_slot_schema: Value,
    requires_state: bool,
    update_hook: Option<Value>,
    created_at: OffsetDateTime,
}

impl TryFrom<AgentRow> for ReleaseAgent {
    type Error = ReleaseError;

    fn try_from(row: AgentRow) -> Result<Self, Self::Error> {
        let policy = row
            .runtime_contract
            .get("policy_ceiling")
            .cloned()
            .ok_or(ReleaseError::InvalidStoredData)
            .and_then(parse_json)?;
        let parameter_schema = parse_json(row.parameter_schema)?;
        let secret_slots =
            serde_json::from_value::<Vec<SecretSlotDeclaration>>(row.secret_slot_schema)
                .map_err(ReleaseError::Serialization)?;
        Ok(Self {
            id: row.id,
            family_id: row.family_id,
            agent_key: row.agent_key,
            display_name: row.display_name,
            policy,
            requires_state: row.requires_state,
            parameter_schema,
            secret_slots,
            update_hook: row.update_hook.map(parse_update_hook).transpose()?,
            created_at: row.created_at,
        })
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, ReleaseError> {
    serde_json::from_value(value).map_err(ReleaseError::Serialization)
}

#[derive(Deserialize)]
struct StoredUpdateHook {
    command: String,
    #[serde(default)]
    arguments: Vec<String>,
    timeout_seconds: u32,
    resources: RuntimePolicy,
}

fn parse_update_hook(value: Value) -> Result<UpdateHook, ReleaseError> {
    let stored = parse_json::<StoredUpdateHook>(value)?;
    let executable = release_domain::ArtifactPath::parse(stored.command)
        .map_err(|_| ReleaseError::InvalidStoredData)?;
    Ok(UpdateHook {
        executable,
        arguments: stored.arguments,
        timeout_seconds: stored.timeout_seconds,
        policy: stored.resources,
    })
}

pub fn parse_state(value: &str) -> Result<ReleaseState, ReleaseError> {
    match value {
        "draft" => Ok(ReleaseState::Draft),
        "published" => Ok(ReleaseState::Published),
        "revoked" => Ok(ReleaseState::Revoked),
        _ => Err(ReleaseError::InvalidStoredData),
    }
}
