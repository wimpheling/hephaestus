//! Complete worker-only immutable original owner observation.

#[path = "vm_owner_history/records.rs"]
mod records;
#[cfg(test)]
#[path = "vm_owner_history/tests.rs"]
mod tests;

use crate::errors::storage;
use records::{Original, classify};
use run_domain::VmOwnerHistory;
use run_orchestrator::RepositoryError;
use sqlx::{PgPool, Postgres, Transaction};
use vm_trait::VmProviderOwnerScope;

/// Concrete worker adapter; contains no pool getter or acquisition operation.
///
/// Queries all original Runs without lifecycle, host or project filters. The
/// result is observed comparison data, never a root/bootstrap/fence authority.
pub struct PgVmOwnerHistoryReader {
    worker: PgPool,
}

impl PgVmOwnerHistoryReader {
    /// Retains the actual dedicated worker pool; each observation checks its role.
    #[must_use]
    pub const fn new(worker: PgPool) -> Self {
        Self { worker }
    }

    /// Compares complete original owner history with trusted configured identity.
    ///
    /// Closed, cancelled, cleaned and retired producer history remains included.
    /// Unattributable global originals hold even when another host is configured.
    /// The RRRO transaction is explicitly rolled back on ordinary success/error
    /// paths; the primary error wins if rollback also fails. Caller cancellation
    /// supplies no awaited-rollback guarantee or persistent observation proof.
    ///
    /// # Errors
    /// Rejects a noncanonical expected VM UUID, nonworker role or database failure.
    pub async fn observe(
        &self,
        expected: &VmProviderOwnerScope,
    ) -> Result<VmOwnerHistory, RepositoryError> {
        records::namespace(expected.namespace())?;
        let mut tx = self.worker.begin().await.map_err(storage)?;
        let result = observe(&mut tx, expected).await;
        let rollback = tx.rollback().await.map_err(storage);
        match result {
            Err(primary) => Err(primary),
            Ok(history) => rollback.map(|()| history),
        }
    }
}

async fn observe(
    tx: &mut Transaction<'_, Postgres>,
    expected: &VmProviderOwnerScope,
) -> Result<VmOwnerHistory, RepositoryError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    sqlx::query("SET LOCAL search_path = pg_catalog, public, pg_temp")
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if role != "hephaestus_worker" {
        return Err(RepositoryError::InvalidData(
            "VM owner history requires worker role",
        ));
    }
    let originals = sqlx::query_as::<_, Original>(include_str!("vm_owner_history/originals.sql"))
        .fetch_all(&mut **tx)
        .await
        .map_err(storage)?;
    Ok(classify(&originals, expected))
}
