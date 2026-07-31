//! `PostgreSQL` persistence adapter for isolated build execution.
//!
//! Build execution ports are being staged here while filesystem, Git, VM, and
//! artifact effects remain owned by `build-orchestrator`.

use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision};
use build_orchestrator::{
    BuildInput, BuildRepository, BuildRepositoryError, ClaimedBuild, FinalizationBuild,
    RecoverableBuild,
};
use identity_domain::{RequestId, UserId};
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseId, ReleaseVersion};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;
use vm_trait::VmExit;

/// `PostgreSQL` implementation of the provider-neutral build persistence port.
#[derive(Clone)]
pub struct PgBuildRepository {
    pool: PgPool,
}

impl PgBuildRepository {
    /// Creates a repository backed by an existing `PostgreSQL` pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> BuildRepositoryError {
    BuildRepositoryError::Storage(Box::new(error))
}

#[derive(Debug, FromRow)]
struct InputRow {
    repository_id: Uuid,
    source_commit: String,
    source_ref: String,
    state: String,
    config: Value,
    created_by: Option<Uuid>,
}

fn claimed(
    id: BuildRequestId,
    row: InputRow,
    release_id: ReleaseId,
    agent: ReleaseAgentId,
    version: ReleaseVersion,
) -> Result<ClaimedBuild, BuildRepositoryError> {
    let config: agent_config::AgentConfig =
        serde_json::from_value(row.config).map_err(|_| BuildRepositoryError::InvalidData)?;
    Ok(ClaimedBuild {
        input: BuildInput {
            id,
            repository_id: row.repository_id,
            source_commit: row.source_commit,
            source_ref: row.source_ref,
            build: config.build.ok_or(BuildRepositoryError::InvalidData)?,
        },
        release_id,
        release_agent_id: agent,
        release_version: version,
    })
}

#[async_trait]
impl BuildRepository for PgBuildRepository {
    async fn recoverable(&self) -> Result<Vec<RecoverableBuild>, BuildRepositoryError> {
        let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT build_request_id, vm_id FROM build_executions WHERE state IN ('claimed','running') ORDER BY updated_at, build_request_id")
            .fetch_all(&self.pool).await.map_err(storage)?;
        Ok(rows
            .into_iter()
            .map(|(id, vm_id)| RecoverableBuild {
                id: BuildRequestId::from_uuid(id),
                vm_id,
            })
            .collect())
    }

    async fn finalizing(&self) -> Result<Vec<BuildRequestId>, BuildRepositoryError> {
        let rows: Vec<Uuid> = sqlx::query_scalar("SELECT build_request_id FROM build_executions WHERE state IN ('sealed','imported') ORDER BY updated_at, build_request_id")
            .fetch_all(&self.pool).await.map_err(storage)?;
        Ok(rows.into_iter().map(BuildRequestId::from_uuid).collect())
    }

    async fn reset_after_cleanup(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        sqlx::query("UPDATE build_executions SET state='claimed', exit_code=NULL, exit_signal=NULL, logs='[]', metrics='[]', started_at=NULL, updated_at=now() WHERE build_request_id=$1 AND state IN ('claimed','running')")
            .bind(id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(storage)?;
        Ok(())
    }

    async fn completed(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<(ReleaseId, ReleaseAgentId, ReleaseVersion, usize)>, BuildRepositoryError>
    {
        let row: Option<(Uuid, Uuid, String, Value)> = sqlx::query_as("SELECT release_id, release_agent_id, release_version, artifact_manifest FROM build_executions WHERE build_request_id = $1 AND state = 'drafted'")
            .bind(id.as_uuid()).fetch_optional(&self.pool).await.map_err(storage)?;
        row.map(|(release, agent, version, manifest)| {
            let count = manifest
                .as_array()
                .ok_or(BuildRepositoryError::InvalidData)?
                .len();
            Ok((
                ReleaseId::from_uuid(release),
                ReleaseAgentId::from_uuid(agent),
                ReleaseVersion::parse(version).map_err(|_| BuildRepositoryError::InvalidData)?,
                count,
            ))
        })
        .transpose()
    }

    async fn finalization(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<FinalizationBuild>, BuildRepositoryError> {
        let row: Option<(String, Uuid, Uuid, String, Option<Value>)> = sqlx::query_as("SELECT state, release_id, release_agent_id, release_version, artifact_manifest FROM build_executions WHERE build_request_id = $1 AND state IN ('sealed','imported')")
            .bind(id.as_uuid()).fetch_optional(&self.pool).await.map_err(storage)?;
        let Some((state, release, agent, version, artifact_manifest)) = row else {
            return Ok(None);
        };
        let input: InputRow = sqlx::query_as("SELECT request.repository_id, request.source_commit, request.source_ref, request.state, request.created_by, revision.config FROM build_requests AS request JOIN LATERAL (SELECT config FROM agent_config_revisions WHERE repository_id = request.repository_id AND commit_sha = request.source_commit AND status = 'valid' ORDER BY created_at DESC LIMIT 1) AS revision ON true WHERE request.id = $1")
            .bind(id.as_uuid()).fetch_optional(&self.pool).await.map_err(storage)?.ok_or(BuildRepositoryError::Unavailable)?;
        Ok(Some(FinalizationBuild {
            state,
            claimed: claimed(
                id,
                input,
                ReleaseId::from_uuid(release),
                ReleaseAgentId::from_uuid(agent),
                ReleaseVersion::parse(version).map_err(|_| BuildRepositoryError::InvalidData)?,
            )?,
            artifact_manifest,
        }))
    }

    async fn claim(&self, id: BuildRequestId) -> Result<ClaimedBuild, BuildRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let row: InputRow = sqlx::query_as("SELECT request.repository_id, request.source_commit, request.source_ref, request.state, request.created_by, revision.config FROM build_requests AS request JOIN LATERAL (SELECT config FROM agent_config_revisions WHERE repository_id = request.repository_id AND commit_sha = request.source_commit AND status = 'valid' ORDER BY created_at DESC LIMIT 1) AS revision ON true WHERE request.id = $1 FOR UPDATE OF request")
            .bind(id.as_uuid()).fetch_optional(&mut *tx).await.map_err(storage)?.ok_or(BuildRepositoryError::Unavailable)?;
        let actor = row.created_by.ok_or(BuildRepositoryError::Unauthorized)?;
        sqlx::query("SELECT set_config('hephaestus.actor_id', $1, true), set_config('hephaestus.subject_type', 'user', true), set_config('hephaestus.request_id', $2, true)").bind(actor.to_string()).bind(id.to_string()).execute(&mut *tx).await.map_err(storage)?;
        let object = ObjectRef::new(ObjectType::Build, id.as_uuid());
        let decision = PostgresMelangeAuthorizer
            .check(
                &mut tx,
                Subject::User(UserId::from_uuid(actor)),
                Permission::CanExecute,
                object,
            )
            .await
            .map_err(|_| BuildRepositoryError::Authorization)?;
        audit_decision(
            &mut tx,
            UserId::from_uuid(actor),
            Permission::CanExecute,
            object,
            decision,
            RequestId::from_uuid(id.as_uuid()),
        )
        .await
        .map_err(storage)?;
        if !decision.is_allowed() {
            tx.commit().await.map_err(storage)?;
            return Err(BuildRepositoryError::Unauthorized);
        }
        if row.state == "running" {
            let existing: Option<(Uuid, Uuid, String, String)> = sqlx::query_as("SELECT release_id, release_agent_id, release_version, state FROM build_executions WHERE build_request_id = $1").bind(id.as_uuid()).fetch_optional(&mut *tx).await.map_err(storage)?;
            let Some((release, agent, version, state)) = existing else {
                return Err(BuildRepositoryError::InvalidData);
            };
            if state != "claimed" {
                return Err(BuildRepositoryError::AlreadyClaimed);
            }
            tx.commit().await.map_err(storage)?;
            return claimed(
                id,
                row,
                ReleaseId::from_uuid(release),
                ReleaseAgentId::from_uuid(agent),
                ReleaseVersion::parse(version).map_err(|_| BuildRepositoryError::InvalidData)?,
            );
        }
        if row.state != "queued" {
            return Err(BuildRepositoryError::AlreadyClaimed);
        }
        let release = ReleaseId::new();
        let agent = ReleaseAgentId::new();
        let version = ReleaseVersion::parse(format!(
            "build-{}",
            &id.as_uuid().simple().to_string()[..16]
        ))
        .map_err(|_| BuildRepositoryError::InvalidData)?;
        sqlx::query("INSERT INTO build_executions (build_request_id, vm_id, release_id, release_agent_id, release_version, state) VALUES ($1,$2,$3,$4,$5,'claimed')").bind(id.as_uuid()).bind(format!("build-{id}")).bind(release.as_uuid()).bind(agent.as_uuid()).bind(version.as_str()).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("UPDATE build_requests SET state = 'running', started_at = now() WHERE id = $1 AND state = 'queued'").bind(id.as_uuid()).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        claimed(id, row, release, agent, version)
    }

    async fn mark_running(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        sqlx::query("UPDATE build_executions SET state='running', started_at=now(), updated_at=now() WHERE build_request_id=$1 AND state='claimed'").bind(id.as_uuid()).execute(&self.pool).await.map_err(storage)?;
        Ok(())
    }
    async fn mark_sealed(
        &self,
        id: BuildRequestId,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("UPDATE build_executions SET state='sealed', exit_code=$2, exit_signal=$3, logs=$4, metrics=$5, sealed_at=now(), updated_at=now() WHERE build_request_id=$1 AND state='running'").bind(id.as_uuid()).bind(exit.code).bind(exit.signal).bind(Value::Array(logs.to_vec())).bind(Value::Array(metrics.to_vec())).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("UPDATE build_requests SET state='importing' WHERE id=$1")
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)
    }
    async fn mark_imported(
        &self,
        id: BuildRequestId,
        artifacts: &[Value],
    ) -> Result<(), BuildRepositoryError> {
        sqlx::query("UPDATE build_executions SET state='imported', artifact_manifest=$2, imported_at=now(), updated_at=now() WHERE build_request_id=$1 AND state='sealed'").bind(id.as_uuid()).bind(Value::Array(artifacts.to_vec())).execute(&self.pool).await.map_err(storage)?;
        Ok(())
    }
    async fn mark_drafted(&self, id: BuildRequestId) -> Result<(), BuildRepositoryError> {
        sqlx::query("UPDATE build_executions SET state='drafted', completed_at=now(), updated_at=now() WHERE build_request_id=$1 AND state='imported'").bind(id.as_uuid()).execute(&self.pool).await.map_err(storage)?;
        Ok(())
    }
    async fn fail(
        &self,
        id: BuildRequestId,
        code: &str,
        exit_code: Option<i32>,
        exit_signal: Option<i32>,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildRepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("UPDATE build_executions SET state='failed', exit_code=$2, exit_signal=$3, failure_code=$4, logs=$5, metrics=$6, completed_at=now(), updated_at=now() WHERE build_request_id=$1 AND state <> 'drafted'").bind(id.as_uuid()).bind(exit_code).bind(exit_signal).bind(code).bind(Value::Array(logs.to_vec())).bind(Value::Array(metrics.to_vec())).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("UPDATE build_requests SET state='failed', diagnostics=jsonb_build_array(jsonb_build_object('code',$2)), completed_at=now() WHERE id=$1 AND state <> 'succeeded'").bind(id.as_uuid()).bind(code).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)
    }
}
