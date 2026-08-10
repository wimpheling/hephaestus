//! Authorized release read operations used by the transport layer.
#![allow(clippy::unused_async)] // Query methods retain async transport contracts while adapter SQL is introduced.

use crate::build::BuildApplication;
use crate::build::BuildView;
use agent_config::SecretSlotDeclaration;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use release_domain::{ParameterDeclaration, UpdateHook};
use release_domain::{ReleaseVersion, RuntimePolicy};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct ReleasePage {
    pub size: i64,
    pub after: Option<Uuid>,
}

pub type ReleaseState = release_domain::ReleaseState;

pub fn encode_cursor(value: Uuid) -> String {
    value.to_string()
}
pub fn decode_cursor(value: &str) -> Option<Uuid> {
    Uuid::parse_str(value).ok()
}

pub struct ReleasePageResult {
    pub releases: Vec<ReleaseSummary>,
    pub next: Option<Uuid>,
}

pub struct ReleaseSummary {
    pub id: Uuid,
    pub version: String,
    pub state: ReleaseState,
    pub source_commit: String,
    pub source_ref: String,
    pub build_request_id: Uuid,
    pub created_at: OffsetDateTime,
    pub published_at: Option<OffsetDateTime>,
    pub manifest_hash: String,
    pub artifact_count: u32,
    pub agent_count: u32,
}

pub struct ReleaseArtifact {
    pub id: Uuid,
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
}
pub struct ReleaseAgent {
    pub id: Uuid,
    pub family_id: Uuid,
    pub agent_key: String,
    pub display_name: String,
    pub policy: release_domain::RuntimePolicy,
    pub requires_state: bool,
    pub parameter_schema: Vec<ParameterDeclaration>,
    pub secret_slots: Vec<SecretSlotDeclaration>,
    pub update_hook: Option<UpdateHook>,
    pub created_at: OffsetDateTime,
}
pub struct ReleaseDetail {
    pub summary: ReleaseSummary,
    pub build_definition_hash: String,
    pub configuration_hash: String,
    pub revoked_at: Option<OffsetDateTime>,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub build: BuildView,
    pub artifacts: Vec<ReleaseArtifact>,
    pub agents: Vec<ReleaseAgent>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("release not found")]
    NotFound,
    #[error("invalid page")]
    InvalidPage,
    #[error("release version is invalid")]
    InvalidVersion,
    #[error("release version is already in use")]
    Conflict,
    #[error("release lifecycle does not allow this operation")]
    FailedPrecondition,
    #[error("invalid stored data")]
    InvalidStoredData,
    #[error("serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    #[error("persistence failed: {0}")]
    Persistence(#[source] sqlx::Error),
}

pub struct ReleaseApplication {
    pool: PgPool,
}
impl ReleaseApplication {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn list_repository_releases(
        &self,
        identity: &AuthenticatedIdentity,
        repository_id: Uuid,
        page: ReleasePage,
    ) -> Result<ReleasePageResult, ReleaseError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        let rows = sqlx::query_as::<_, ReleaseSummaryRow>(
            "SELECT release.id, release.version, release.state,
                    release.source_commit, release.source_ref,
                    release.build_request_id, release.created_at,
                    release.published_at,
                    encode(release.manifest_hash, 'hex') AS manifest_hash,
                    (SELECT count(*) FROM release_artifacts artifact
                     WHERE artifact.release_id = release.id)::bigint AS artifact_count,
                    (SELECT count(*) FROM release_agents agent
                     WHERE agent.release_id = release.id)::bigint AS agent_count
             FROM releases release
             WHERE release.repository_id = $1
               AND ($2::uuid IS NULL OR (release.created_at, release.id) <
                    (SELECT cursor.created_at, cursor.id
                     FROM releases cursor WHERE cursor.id = $2))
             ORDER BY release.created_at DESC, release.id DESC
             LIMIT $3",
        )
        .bind(repository_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(ReleaseError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)?;
        let size = usize::try_from(page.size).map_err(|_| ReleaseError::InvalidPage)?;
        let has_more = rows.len() > size;
        let releases = rows
            .into_iter()
            .take(size)
            .map(ReleaseSummary::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let next = has_more
            .then(|| releases.last().map(|release| release.id))
            .flatten();
        Ok(ReleasePageResult { releases, next })
    }
    pub async fn get_release(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<ReleaseDetail, ReleaseError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        let row = sqlx::query_as::<_, ReleaseDetailRow>(
            "SELECT release.id, release.version, release.state,
                    release.source_commit, release.source_ref,
                    release.build_request_id, release.created_at,
                    release.published_at,
                    encode(release.manifest_hash, 'hex') AS manifest_hash,
                    encode(release.build_definition_hash, 'hex') AS build_definition_hash,
                    encode(release.configuration_hash, 'hex') AS configuration_hash,
                    release.revoked_at, release.repository_id,
                    repository.name AS repository_name, project.id AS project_id,
                    project.name AS project_name, organization.id AS organization_id,
                    organization.name AS organization_name
             FROM releases release
             JOIN repositories repository ON repository.id = release.repository_id
             JOIN projects project ON project.id = repository.project_id
             JOIN organizations organization ON organization.id = project.organization_id
             WHERE release.id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(ReleaseError::Persistence)?
        .ok_or(ReleaseError::NotFound)?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)?;

        let summary = row.summary()?;
        let build = BuildApplication::new(self.pool.clone())
            .get_build(identity, row.build_request_id)
            .await
            .map_err(map_build_error)?;
        let artifacts = self.artifacts(identity, id).await?;
        let agents = self.agents(identity, id).await?;
        Ok(ReleaseDetail {
            summary,
            build_definition_hash: row.build_definition_hash,
            configuration_hash: row.configuration_hash,
            revoked_at: row.revoked_at,
            repository_id: row.repository_id,
            repository_name: row.repository_name,
            project_id: row.project_id,
            project_name: row.project_name,
            organization_id: row.organization_id,
            organization_name: row.organization_name,
            build,
            artifacts,
            agents,
        })
    }

    pub async fn set_draft_version(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
        version: String,
    ) -> Result<(), ReleaseError> {
        let version = ReleaseVersion::parse(version).map_err(|_| ReleaseError::InvalidVersion)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        ensure_draft(&mut transaction, id).await?;
        let conflict: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM releases candidate
                 WHERE candidate.repository_id = (SELECT repository_id FROM releases WHERE id = $1)
                   AND candidate.version = $2 AND candidate.id <> $1
             )",
        )
        .bind(id)
        .bind(version.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(ReleaseError::Persistence)?;
        if conflict {
            return Err(ReleaseError::Conflict);
        }
        sqlx::query("UPDATE releases SET version = $2 WHERE id = $1 AND state = 'draft'")
            .bind(id)
            .bind(version.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(ReleaseError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)
    }

    pub async fn publish_release(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<(), ReleaseError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        ensure_draft(&mut transaction, id).await?;
        sqlx::query(
            "UPDATE releases
             SET state = 'published', publication_actor_id = $2, published_at = now()
             WHERE id = $1 AND state = 'draft'",
        )
        .bind(id)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(ReleaseError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)
    }

    async fn artifacts(
        &self,
        identity: &AuthenticatedIdentity,
        release_id: Uuid,
    ) -> Result<Vec<ReleaseArtifact>, ReleaseError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT id, path, kind, mode, encode(content_hash, 'hex') AS sha256,
                    size_bytes, media_type
             FROM release_artifacts WHERE release_id = $1
             ORDER BY path, id",
        )
        .bind(release_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(ReleaseError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)?;
        rows.into_iter().map(TryFrom::try_from).collect()
    }

    async fn agents(
        &self,
        identity: &AuthenticatedIdentity,
        release_id: Uuid,
    ) -> Result<Vec<ReleaseAgent>, ReleaseError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        let rows = sqlx::query_as::<_, AgentRow>(
            "SELECT id, family_id, agent_key, display_name, runtime_contract,
                    parameter_schema, secret_slot_schema, requires_state, update_hook,
                    created_at
             FROM release_agents WHERE release_id = $1
             ORDER BY agent_key, id",
        )
        .bind(release_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(ReleaseError::Persistence)?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)?;
        rows.into_iter().map(TryFrom::try_from).collect()
    }
}

fn map_build_error(error: crate::build::BuildError) -> ReleaseError {
    match error {
        crate::build::BuildError::NotFound => ReleaseError::NotFound,
        crate::build::BuildError::InvalidStoredData => ReleaseError::InvalidStoredData,
        crate::build::BuildError::Serialization(error) => ReleaseError::Serialization(error),
        crate::build::BuildError::Persistence(error) => ReleaseError::Persistence(error),
        crate::build::BuildError::FailedPrecondition => ReleaseError::FailedPrecondition,
    }
}

async fn ensure_draft(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<(), ReleaseError> {
    let state = sqlx::query_scalar::<_, String>("SELECT state FROM releases WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(ReleaseError::Persistence)?
        .ok_or(ReleaseError::NotFound)?;
    if state == "draft" {
        Ok(())
    } else {
        Err(ReleaseError::FailedPrecondition)
    }
}

#[derive(FromRow)]
struct ReleaseSummaryRow {
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
struct ReleaseDetailRow {
    id: Uuid,
    version: String,
    state: String,
    source_commit: String,
    source_ref: String,
    build_request_id: Uuid,
    created_at: OffsetDateTime,
    published_at: Option<OffsetDateTime>,
    manifest_hash: String,
    build_definition_hash: String,
    configuration_hash: String,
    revoked_at: Option<OffsetDateTime>,
    repository_id: Uuid,
    repository_name: String,
    project_id: Uuid,
    project_name: String,
    organization_id: Uuid,
    organization_name: String,
}

impl ReleaseDetailRow {
    fn summary(&self) -> Result<ReleaseSummary, ReleaseError> {
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
struct ArtifactRow {
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
struct AgentRow {
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

fn parse_state(value: &str) -> Result<ReleaseState, ReleaseError> {
    match value {
        "draft" => Ok(ReleaseState::Draft),
        "published" => Ok(ReleaseState::Published),
        "revoked" => Ok(ReleaseState::Revoked),
        _ => Err(ReleaseError::InvalidStoredData),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_state;
    use release_domain::ReleaseVersion;

    #[test]
    fn release_state_and_version_contracts_match_product_values() {
        assert!(parse_state("draft").is_ok());
        assert!(parse_state("published").is_ok());
        assert!(parse_state("revoked").is_ok());
        assert!(parse_state("running").is_err());
        for version in ["v1.0.0", "2026.07", "experimental-4"] {
            assert!(ReleaseVersion::parse(version).is_ok());
        }
        assert!(ReleaseVersion::parse("bad/version").is_err());
    }
}
