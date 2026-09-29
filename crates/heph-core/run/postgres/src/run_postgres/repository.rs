use async_trait::async_trait;
use run_domain::{CancelRun, Run, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{EventId, RunId};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;

use super::errors::storage;
use super::model::RunRow;

/// `PostgreSQL` implementation of [`RunRepository`].
#[derive(Clone)]
pub struct PgRunRepository {
    pub(crate) pool: PgPool,
}

impl PgRunRepository {
    /// Creates a repository using an existing connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(super) async fn locked_run(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<RunRow, RepositoryError> {
        sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1 FOR UPDATE")
            .bind(run_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))
    }

    pub(super) async fn append_event_tx(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
        event_type: &str,
        payload: Value,
        occurred_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_events WHERE run_id = $1",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        let event_id = EventId::new();
        sqlx::query(
            "INSERT INTO run_events
             (id, run_id, sequence, event_type, payload, occurred_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(event_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(sequence)
        .bind(event_type)
        .bind(&payload)
        .bind(occurred_at)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
        Ok(())
    }
}

#[async_trait]
impl RunRepository for PgRunRepository {
    async fn create_run(&self, command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        self.create_run_impl(command).await
    }

    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError> {
        self.ensure_runtime_git_provenance_impl(run).await
    }

    async fn get(&self, run_id: RunId) -> Result<Run, RepositoryError> {
        self.load_run(run_id).await
    }

    async fn bind_resources(
        &self,
        run_id: RunId,
        volume_id: Option<runtime_types::VolumeId>,
        lease_id: Option<runtime_types::LeaseId>,
        lease_fencing_token: Option<i64>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        self.bind_resources_impl(run_id, volume_id, lease_id, lease_fencing_token, vm_id)
            .await
    }

    async fn transition(
        &self,
        run_id: RunId,
        next: run_domain::RunState,
        exit: Option<&vm_trait::VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        self.transition_impl(run_id, next, exit, failure).await
    }

    async fn append_vm_event(
        &self,
        run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        self.append_vm_event_impl(run_id, event).await
    }

    async fn request_cancel(&self, command: &CancelRun) -> Result<bool, RepositoryError> {
        self.request_cancel_impl(command).await
    }

    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        self.recoverable_runs_impl().await
    }
}
