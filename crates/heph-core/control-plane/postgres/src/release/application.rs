use super::{
    helpers::{ensure_draft, map_build_error},
    rows::{AgentRow, ArtifactRow, ReleaseDetailRow, ReleaseSummaryRow},
    types::{
        ReleaseAgent, ReleaseArtifact, ReleaseDetail, ReleaseError, ReleasePage, ReleasePageResult,
        ReleaseSummary,
    },
    ui,
};
use crate::build::BuildApplication;
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use release_domain::ReleaseVersion;
use sqlx::PgPool;
use uuid::Uuid;

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
               AND check_permission('user', hephaestus_actor_id(), 'can_read',
                   'release', release.id::text) = 1
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
             WHERE release.id = $1
               AND check_permission('user', hephaestus_actor_id(), 'can_read',
                   'release', release.id::text) = 1",
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
        let ui_descriptors = self.ui_descriptors(identity, id).await?;
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
            ui_descriptors,
        })
    }

    async fn ui_descriptors(
        &self,
        identity: &AuthenticatedIdentity,
        release_id: Uuid,
    ) -> Result<Vec<ui::ReleaseUiDescriptor>, ReleaseError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(ReleaseError::Persistence)?;
        let descriptors = ui::load_release_ui_descriptors(&mut transaction, release_id).await?;
        transaction
            .commit()
            .await
            .map_err(ReleaseError::Persistence)?;
        Ok(descriptors)
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
             FROM release_artifacts
             WHERE release_id = $1
               AND check_permission('user', hephaestus_actor_id(), 'can_read',
                   'release', release_id::text) = 1
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
