use run_domain::Run;
use run_orchestrator::RepositoryError;
use runtime_types::{LeaseId, RunId, VolumeId};

use crate::{PgRunRepository, errors::storage};

impl PgRunRepository {
    pub(crate) async fn bind_profiled_resources(
        &self,
        run_id: RunId,
        volume_id: Option<VolumeId>,
        lease_id: Option<LeaseId>,
        fence: Option<i64>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        self.locked_profiled_run(&mut tx, run_id).await?;
        // Legacy acquisition already committed the exact tuple. Owned binding
        // may first project only the checked singleton LegacyOrigin full set.
        sqlx::query("SELECT assert_profiled_run_bind($1,$2,$3,$4,$5)")
            .bind(run_id.as_uuid())
            .bind(volume_id.map(VolumeId::as_uuid))
            .bind(lease_id.map(LeaseId::as_uuid))
            .bind(fence)
            .bind(vm_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        sqlx::query("UPDATE runs SET vm_id=$2,volume_id=$3,lease_id=$4,lease_fencing_token=$5,updated_at=now() WHERE id=$1")
            .bind(run_id.as_uuid())
            .bind(vm_id)
            .bind(volume_id.map(VolumeId::as_uuid))
            .bind(lease_id.map(LeaseId::as_uuid))
            .bind(fence)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        self.load_run(run_id).await
    }
}
