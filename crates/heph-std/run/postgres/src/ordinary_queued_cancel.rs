//! Worker-owned queued cancellation metadata, without provider IO.

mod context;
mod preparation;
pub use context::Context;

use run_domain::{Run, RunCleanupTarget};
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use vm_trait::VmProviderOwnerScope;

use crate::{PgRunRepository, errors::storage};

pub async fn selected(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
) -> Result<bool, RepositoryError> {
    sqlx::query_scalar("SELECT ordinary_queued_cancel_selected140($1)")
        .bind(run.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)
}

impl PgRunRepository {
    /// Prepares owning-processed ordinary or genuine removal Cancel before consumption.
    ///
    /// Uses the retained trusted configuration and exact configured worker owner.
    /// Freezes only this Run's acquisition and complete zero-resource target;
    /// leaves the instance gate, grants, other Runs and original Start unchanged.
    /// Neither the returned Run nor its target proves physical VM absence or
    /// cleanup completion. A genuine scoped observation remains mandatory.
    ///
    /// # Errors
    /// Rejects unsupported composition, pending/conflicting or unadmitted Cancel,
    /// different scope/configuration, resource history, consumption or lock holds.
    pub async fn prepare_queued_cancel_cleanup(
        &self,
        run: RunId,
        scope: &VmProviderOwnerScope,
    ) -> Result<(Run, RunCleanupTarget), RepositoryError> {
        self.prepare_queued_invocation_cancellation(run, scope)
            .await
    }
}
