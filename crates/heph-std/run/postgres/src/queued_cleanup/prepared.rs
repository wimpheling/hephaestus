//! Prepared queued cleanup reads and transition-only owning selection.

use run_domain::{Run, RunState};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use vm_trait::VmExit;

use super::{configuration_for_queued, context::Context};
use crate::{PgRunRepository, errors::storage, model::RunRow};

impl PgRunRepository {
    pub(crate) async fn load_prepared_queued_cleanup(
        &self,
        run: RunId,
    ) -> Result<Option<Run>, RepositoryError> {
        let Some(scope) = self.cleanup_wake_scope.as_ref() else {
            // Default/public107 and compositions without Wake do no new probe.
            return Ok(None);
        };
        let config = self.cleanup_wake_configuration(scope)?;
        let mut tx = self.cleanup_wake_transaction(true).await?;
        let Some(config) = configuration_for_queued(&mut tx, run, config.as_deref()).await? else {
            tx.commit().await.map_err(storage)?;
            return Ok(None);
        };
        let context = Context::discover(&mut tx, run, scope, config).await?;
        let row = context.load_prepared(&mut tx, run, scope, config).await?;
        // Only positive closed zero-target proof, never candidate/row labels.
        let prepared = row.into_checked_invocation()?;
        tx.commit().await.map_err(storage)?;
        Ok(Some(prepared))
    }

    pub(crate) async fn lock_prepared_queued_cleanup_transition(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: RunId,
        next: RunState,
        exit: Option<&VmExit>,
        failure: Option<&str>,
    ) -> Result<Option<RunRow>, RepositoryError> {
        let Some(scope) = self.cleanup_wake_scope.as_ref() else {
            return Ok(None);
        };
        let config = self.cleanup_wake_configuration(scope)?;
        // The caller owns the original fresh transaction, before any parent lock.
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        if !worker {
            return Err(RepositoryError::InvalidData(
                "queued cleanup transition requires the configured worker role",
            ));
        }
        let Some(config) = configuration_for_queued(tx, run, config.as_deref()).await? else {
            return Ok(None);
        };
        let writable_rc: bool = sqlx::query_scalar(
            "SELECT current_setting('transaction_isolation')='read committed' AND current_setting('transaction_read_only')='off'",
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        if !writable_rc {
            return Err(RepositoryError::InvalidData(
                "queued cleanup requires writable read-committed transaction",
            ));
        }
        if !matches!(next, RunState::Cancelled | RunState::CleaningUp)
            || exit.is_some()
            || failure.is_some()
        {
            return Err(RepositoryError::InvalidData(
                "queued cleanup only permits cancellation and cleanup without exit or failure",
            ));
        }
        let context = Context::discover(tx, run, scope, config).await?;
        // The discovery facade also permits a candidate. Prove strict prepared
        // work before even the early waits; never enter general consumption.
        context.load_prepared(tx, run, scope, config).await?;
        context.reserve_and_lock(tx, run, scope, config).await?;
        // Fresh strict reader after owning locks; no stale pre-lock Run copy.
        let row = context.load_prepared(tx, run, scope, config).await?;
        Ok(Some(row))
    }
}
