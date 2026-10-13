//! Fresh hook placement and exact replay; immutable producer identity is retained.

use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementScope};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{ReleaseService, ReleaseServiceError};

impl ReleaseService {
    pub(super) async fn lock_legacy_hook_parent(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        update: Uuid,
    ) -> Result<(), ReleaseServiceError> {
        if self.legacy_vm_scope.is_some() {
            sqlx::query("SELECT require_run_cleanup_worker()")
                .execute(&mut **tx)
                .await?;
            sqlx::query("SELECT consumer.id FROM agent_instances consumer JOIN agent_updates update ON update.instance_id=consumer.id WHERE update.id=$1 FOR UPDATE OF consumer")
                .bind(update).execute(&mut **tx).await?;
        }
        Ok(())
    }

    /// Configures the expected actual managed VM provider for fresh Legacy hooks.
    ///
    /// Explicit `LegacyScalar` selection is required. This does not adopt history
    /// or confer source permissions, VM absence evidence or canonical receipts.
    #[must_use]
    pub fn with_legacy_vm_placement(mut self, scope: LegacyVmPlacementScope) -> Self {
        self.legacy_vm_scope = Some(scope);
        self
    }

    pub(super) fn require_legacy_placement_profile(&self) -> Result<(), ReleaseServiceError> {
        if self.legacy_vm_scope.is_some()
            && self.execution_profile != Some(FreshRunExecutionProfile::LegacyScalar)
        {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        Ok(())
    }

    pub(super) async fn stamp_legacy_hook_placement(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: Uuid,
    ) -> Result<(), ReleaseServiceError> {
        let Some(scope) = &self.legacy_vm_scope else {
            return Ok(());
        };
        let namespace = scope
            .provider_scope()
            .namespace()
            .parse::<Uuid>()
            .map_err(|_| ReleaseServiceError::InvalidUpdateLifecycle)?;
        sqlx::query("SELECT record_legacy_vm_placement($1,$2,$3,'update_hook')")
            .bind(run)
            .bind(namespace)
            .bind(scope.provider_scope().host_id())
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub(super) async fn require_legacy_hook_replay(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: Uuid,
    ) -> Result<(), ReleaseServiceError> {
        let Some(scope) = &self.legacy_vm_scope else {
            return Ok(());
        };
        // New configured replay remains worker-only before placement lookup.
        sqlx::query("SELECT require_run_cleanup_worker()")
            .execute(&mut **tx)
            .await?;
        let namespace = scope
            .provider_scope()
            .namespace()
            .parse::<Uuid>()
            .map_err(|_| ReleaseServiceError::InvalidUpdateLifecycle)?;
        let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runs execution JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id WHERE execution.id=$1 AND plan.provider_namespace=$2 AND plan.host_id=$3 AND plan.producer_kind='update_hook' AND legacy_vm_placement_matches(execution,plan))")
            .bind(run).bind(namespace).bind(scope.provider_scope().host_id())
            .fetch_one(&mut **tx).await?;
        if !exact {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        Ok(())
    }
}
