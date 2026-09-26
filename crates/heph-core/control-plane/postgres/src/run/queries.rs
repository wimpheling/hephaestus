//! Authorized run provenance and project listing.

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

use super::model::{
    AuthorizationProvenance, HttpsUse, Page, PageResult, RunApplication, RunError, RunProvenance,
    RunSummary,
};

impl RunApplication {
    pub async fn get_run_provenance(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
        page: Page,
    ) -> Result<RunProvenance, RunError> {
        if !(1..=200).contains(&page.size) {
            return Err(RunError::InvalidPage);
        }
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(RunError::Persistence)?;
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM runs WHERE id = $1 AND check_permission('user', hephaestus_actor_id(), 'can_read', 'run', id::text) = 1")
            .bind(id).fetch_optional(&mut *tx).await.map_err(RunError::Persistence)?
            .ok_or(RunError::NotFound)?;
        let snapshot = sqlx::query_as::<_, AuthorizationProvenance>(
            "SELECT id, authorization_model_version, encode(normalized_hash, 'hex') AS normalized_hash FROM run_authorization_snapshots WHERE run_id = $1"
        ).bind(id).fetch_optional(&mut *tx).await.map_err(RunError::Persistence)?;
        let mut values =
            sqlx::query_as::<_, HttpsUse>("SELECT * FROM inspect_run_https_uses($1, $2, $3)")
                .bind(id)
                .bind(page.after)
                .bind(i32::try_from(page.size + 1).map_err(|_| RunError::InvalidPage)?)
                .fetch_all(&mut *tx)
                .await
                .map_err(RunError::Persistence)?;
        tx.commit().await.map_err(RunError::Persistence)?;
        let size = usize::try_from(page.size).map_err(|_| RunError::InvalidPage)?;
        let more = values.len() > size;
        values.truncate(size);
        let next = more
            .then(|| values.last())
            .flatten()
            .map(|row| row.id.to_string());
        Ok(RunProvenance {
            snapshot,
            uses: PageResult { values, next },
        })
    }

    pub const fn new(pool: PgPool, result_artifact_root: PathBuf) -> Self {
        Self {
            pool,
            result_artifact_root,
        }
    }

    pub async fn list_project_runs(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<RunSummary>, RunError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(RunError::Persistence)?;
        let mut values = sqlx::query_as::<_, RunSummary>(
            "SELECT run.id, run.state, run.outcome, run.run_kind, run.updated_at,
                    instance.id AS instance_id, instance.name AS instance_name,
                    request.repository_id, repository.name AS repository_name,
                    request.commit_sha, request.git_ref, release.id AS release_id,
                    release.version AS release_version, run.instance_revision_id
             FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id
             LEFT JOIN run_requests request ON request.run_id = run.id
             LEFT JOIN repositories repository ON repository.id = request.repository_id
             JOIN releases release ON release.id = run.release_id
             WHERE instance.project_id = $1 AND ($2::uuid IS NULL OR (run.created_at, run.id) <
                 (SELECT cursor.created_at, cursor.id FROM runs cursor WHERE cursor.id = $2))
             ORDER BY run.created_at DESC, run.id DESC LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(RunError::Persistence)?;
        tx.commit().await.map_err(RunError::Persistence)?;
        let size = usize::try_from(page.size).map_err(|_| RunError::InvalidPage)?;
        let has_more = values.len() > size;
        values.truncate(size);
        let next = has_more
            .then(|| values.last())
            .flatten()
            .map(|row| row.id.to_string());
        Ok(PageResult { values, next })
    }
}
