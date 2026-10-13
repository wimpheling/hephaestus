//! Qualified original comparison data and strict prepared cleanup boundaries.

use run_domain::RunCleanupHostId;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{cleanup_wake::qualification_error, errors::storage, model::RunRow};

#[derive(sqlx::FromRow)]
pub struct DrainContext {
    instance_id: Uuid,
    original: Value,
}

#[derive(sqlx::FromRow)]
struct Reservation {
    lock_class: i32,
    command_mutex: Option<i64>,
    first_mutex: Option<i32>,
    second_mutex: Option<i32>,
}

pub enum Context {
    Drain(DrainContext),
    Ordinary(crate::ordinary_queued_cancel::Context),
}

impl Context {
    // This facade derives DATA then positively qualifies current actual config,
    // genuine115/original processed Cancel/zero history/candidate or prepared work.
    // Strict prepared proof must follow; context alone never permits hydration.
    pub async fn discover(
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<Self, RepositoryError> {
        if crate::ordinary_queued_cancel::selected(tx, run).await? {
            return crate::ordinary_queued_cancel::Context::discover(tx, run, scope, config)
                .await
                .map(Self::Ordinary);
        }
        sqlx::query_as("SELECT * FROM discover_queued_run_cleanup_context119($1,$2,$3,$4)")
            .bind(run.as_uuid())
            .bind(scope.provider_namespace())
            .bind(scope.host_id())
            .bind(config)
            .fetch_one(&mut **tx)
            .await
            .map_err(qualification_error)
            .map(Self::Drain)
    }

    pub async fn load_prepared(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<RunRow, RepositoryError> {
        let drain = match self {
            Self::Drain(drain) => drain,
            Self::Ordinary(ordinary) => {
                return ordinary.load_prepared(tx, run, scope, config).await;
            }
        };
        sqlx::query_as("SELECT * FROM load_invocation119_cleanup_only($1,$2,$3,$4,$5,$6)")
            .bind(run.as_uuid())
            .bind(drain.instance_id)
            .bind(&drain.original)
            .bind(scope.provider_namespace())
            .bind(scope.host_id())
            .bind(config)
            .fetch_one(&mut **tx)
            .await
            .map_err(qualification_error)
    }

    /// Requalifies candidate or prepared replay under the existing owning locks.
    pub async fn reserve_and_lock_preparing(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<(), RepositoryError> {
        self.reserve_and_lock_phase(tx, run, scope, config, true)
            .await
    }

    pub async fn reserve_and_lock(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
    ) -> Result<(), RepositoryError> {
        self.reserve_and_lock_phase(tx, run, scope, config, false)
            .await
    }

    async fn reserve_and_lock_phase(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        scope: &RunCleanupHostId,
        config: &[u8],
        preparing: bool,
    ) -> Result<(), RepositoryError> {
        let drain = match self {
            Self::Drain(drain) => drain,
            Self::Ordinary(ordinary) => {
                return ordinary
                    .reserve_and_lock(
                        tx,
                        run,
                        scope,
                        config,
                        if preparing { "preparing" } else { "prepared" },
                    )
                    .await;
            }
        };
        let rows: Vec<Reservation> = sqlx::query_as(
            "SELECT * FROM queued_run_cleanup_reservations119($1,$2,$3,$4,$5,$6) ORDER BY lock_class,command_mutex,first_mutex,second_mutex",
        )
        .bind(run.as_uuid())
        .bind(drain.instance_id)
        .bind(&drain.original)
        .bind(scope.provider_namespace())
        .bind(scope.host_id())
        .bind(config)
        .fetch_all(&mut **tx)
        .await
        .map_err(qualification_error)?;
        if rows.is_empty() {
            return Err(RepositoryError::InvalidData(
                "queued cleanup reservations absent",
            ));
        }
        // Complete sorted early waits precede all parent rows. No late waits.
        for row in rows {
            match (
                row.lock_class,
                row.command_mutex,
                row.first_mutex,
                row.second_mutex,
            ) {
                (0, Some(command), None, None) => {
                    sqlx::query("SELECT pg_advisory_xact_lock($1)")
                        .bind(command)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage)?;
                }
                (1, None, Some(first), Some(second)) => {
                    sqlx::query("SELECT pg_advisory_xact_lock($1,$2)")
                        .bind(first)
                        .bind(second)
                        .execute(&mut **tx)
                        .await
                        .map_err(storage)?;
                }
                _ => {
                    return Err(RepositoryError::InvalidData(
                        "queued cleanup reservation shape differs",
                    ));
                }
            }
        }
        // ff4d guards writable RC before raw TRY/parent/late NOWAIT locks and
        // requalifies the supplied original DATA/current actual config afterward.
        let lock = if preparing {
            "SELECT lock_prepare_queued_run_cleanup_wake119($1,$2,$3,$4,$5,$6)"
        } else {
            "SELECT lock_invocation119_cleanup_only($1,$2,$3,$4,$5,$6)"
        };
        sqlx::query(lock)
            .bind(run.as_uuid())
            .bind(drain.instance_id)
            .bind(&drain.original)
            .bind(scope.provider_namespace())
            .bind(scope.host_id())
            .bind(config)
            .execute(&mut **tx)
            .await
            .map_err(qualification_error)?;
        Ok(())
    }
}
