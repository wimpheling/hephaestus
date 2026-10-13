//! Read-only live worker provenance and exact immutable capability ceilings.

use super::super::{agent_repository::PgRuntimeSessionRepository, common::storage};
use capability_domain::AuthorizationSnapshot;
use run_domain::Run;
use runtime_authority::{LiveRunSnapshotPins, LiveRunSnapshotRequest, RuntimeAuthorityError};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(in crate::runtime_authority) struct Source {
    pub actor_id: Uuid,
    pub request_id: Uuid,
}

impl PgRuntimeSessionRepository {
    pub(in crate::runtime_authority) async fn invocation_source(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request: &LiveRunSnapshotRequest,
    ) -> Result<Source, RuntimeAuthorityError> {
        let pins = request.pins();
        sqlx::query_as("SELECT actor_id,request_id FROM read_invocation119_authority_context($1,$2,$3) WHERE instance_id=$4 AND revision_id=$5 AND release_id=$6 AND release_agent_id=$7 AND command_id=$8")
            .bind(pins.run()).bind(self.invocation_config_bytes()?)
            .bind(request.ceiling().is_some()).bind(pins.instance()).bind(pins.revision())
            .bind(pins.release()).bind(pins.release_agent()).bind(pins.command())
            .fetch_optional(&mut **tx).await.map_err(storage)?
            .ok_or(RuntimeAuthorityError::Unsupported)
    }

    pub(in crate::runtime_authority) async fn invocation_live_snapshot(
        &self,
        request: &LiveRunSnapshotRequest,
        model: &str,
        now: OffsetDateTime,
    ) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
        self.invocation_config_bytes()?;
        let mut tx = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        self.invocation_source(&mut tx, request).await?;
        let snapshot =
            super::super::agent_live_snapshot::ceiling::resolve(&mut tx, request.pins(), model)
                .await?;
        if let Some(expected) = request.ceiling() {
            super::super::agent_live_snapshot::session::validate(
                &mut tx,
                request.pins(),
                *expected,
                snapshot.normalized_hash(),
                now,
            )
            .await?;
        }
        tx.commit().await.map_err(storage)?;
        Ok(snapshot)
    }

    pub(in crate::runtime_authority) async fn invocation_snapshot(
        &self,
        run: &Run,
        model: &str,
    ) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
        let pins = pins(run)?;
        let snapshot = self
            .invocation_live_snapshot(
                &LiveRunSnapshotRequest::pre_issuance(pins),
                model,
                OffsetDateTime::now_utc(),
            )
            .await?;
        Ok(snapshot)
    }

    pub(in crate::runtime_authority) async fn invocation_live_authorized(
        &self,
        run: &Run,
        snapshot: &AuthorizationSnapshot,
    ) -> Result<bool, RuntimeAuthorityError> {
        let pins = pins(run)?;
        let request = match self.recorded_live_session_ceiling(pins).await? {
            Some(ceiling) => LiveRunSnapshotRequest::issued(pins, ceiling)?,
            None => LiveRunSnapshotRequest::pre_issuance(pins),
        };
        let fresh = self
            .invocation_live_snapshot(
                &request,
                snapshot.authorization_model_version(),
                OffsetDateTime::now_utc(),
            )
            .await?;
        Ok(fresh == *snapshot)
    }
}

fn pins(run: &Run) -> Result<LiveRunSnapshotPins, RuntimeAuthorityError> {
    LiveRunSnapshotPins::new(
        run.id.as_uuid(),
        run.instance_id.as_uuid(),
        run.instance_revision_id.as_uuid(),
        run.release_id.as_uuid(),
        run.release_agent_id.as_uuid(),
        run.command_id.as_uuid(),
    )
}
