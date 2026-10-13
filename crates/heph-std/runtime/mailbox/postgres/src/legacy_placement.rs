//! Prospective placement at fresh mailbox birth; accepted payloads stay opaque.

use mailbox_dispatch::MailboxDispatchStoreError;
use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementScope};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{PostgresMailboxRepository, errors::dispatch_error};

impl PostgresMailboxRepository {
    pub(super) async fn lock_legacy_placement_parent(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        event: Uuid,
    ) -> Result<(), MailboxDispatchStoreError> {
        if self.legacy_vm_scope.is_some() {
            // New managed producers lock the consumer before the delivery and
            // fresh Run. The unconfigured historical query sequence is exact.
            sqlx::query("SELECT consumer.id FROM agent_instances consumer JOIN mailbox_events event ON event.instance_id=consumer.id WHERE event.id=$1 FOR UPDATE OF consumer")
                .bind(event).execute(&mut **tx).await.map_err(dispatch_error)?;
        }
        Ok(())
    }

    /// Configures the actual managed Run provider's expected placement.
    ///
    /// Requires explicit `LegacyScalar` selection. No existing delivery or Run is
    /// adopted. The unconfigured producer preserves its original SQL shape.
    #[must_use]
    pub fn with_legacy_vm_placement(mut self, scope: LegacyVmPlacementScope) -> Self {
        self.legacy_vm_scope = Some(scope);
        self
    }

    pub(super) fn require_legacy_placement_profile(&self) -> Result<(), MailboxDispatchStoreError> {
        if self.legacy_vm_scope.is_some()
            && self.execution_profile != Some(FreshRunExecutionProfile::LegacyScalar)
        {
            return Err(MailboxDispatchStoreError(
                "Legacy placement requires explicit `LegacyScalar` producer".into(),
            ));
        }
        Ok(())
    }

    pub(super) async fn stamp_legacy_placement(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: Uuid,
    ) -> Result<(), MailboxDispatchStoreError> {
        let Some(scope) = &self.legacy_vm_scope else {
            return Ok(());
        };
        sqlx::query("SELECT record_legacy_vm_placement($1,$2,$3,'mailbox')")
            .bind(run)
            .bind(
                scope
                    .provider_scope()
                    .namespace()
                    .parse::<Uuid>()
                    .map_err(dispatch_error)?,
            )
            .bind(scope.provider_scope().host_id())
            .execute(&mut **tx)
            .await
            .map_err(dispatch_error)?;
        Ok(())
    }
}
