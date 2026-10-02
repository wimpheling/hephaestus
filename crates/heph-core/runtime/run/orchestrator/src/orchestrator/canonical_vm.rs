//! Worker observations require exact provider ownership before and after IO.

use run_domain::{RunCleanupReceipt, RunCleanupTarget, RunCleanupVmObservation};
use runtime_types::RunId;
use std::sync::Arc;
use time::OffsetDateTime;
use vm_trait::{VmError, VmInstance, VmProviderOwnerScope};

use super::{
    OrchestratorError, RunOrchestrator,
    canonical_cleanup::{checked, host, invalid},
};

impl RunOrchestrator {
    pub(super) fn validate_cleanup_scope(
        &self,
        target: &RunCleanupTarget,
    ) -> Result<VmProviderOwnerScope, OrchestratorError> {
        let scope = self.provider.owner_scope()?;
        if target.vm_target().host() != Some(&host(&scope)?) {
            return Err(invalid(
                "cleanup target has unresolved or different provider ownership",
            ));
        }
        if target.vm_target().vm_id().is_none() {
            return Err(invalid("cleanup target has no proven VM identity"));
        }
        Ok(scope)
    }

    pub(super) async fn confirm_canonical_guest_cleanup(
        &self,
        run_id: RunId,
        instance: Option<Arc<dyn VmInstance>>,
    ) -> Result<RunCleanupReceipt, OrchestratorError> {
        let cleanup = self
            .canonical_cleanup
            .as_ref()
            .ok_or_else(|| invalid("canonical cleanup is not configured"))?;
        let target = cleanup.repository.begin_cleanup(run_id).await?;
        let scope = self.validate_cleanup_scope(&target)?;
        if let Some(receipt) = cleanup.repository.recorded_cleanup_receipt(run_id).await? {
            checked(receipt.matches_target(&target))?;
            return Ok(receipt);
        }
        let vm_id = target
            .vm_target()
            .vm_id()
            .ok_or_else(|| invalid("cleanup target has no proven VM identity"))?;
        let instance = match instance {
            Some(instance) => Some(instance),
            None => self.active.lock().await.get(&run_id).cloned(),
        };
        tokio::time::timeout(self.cleanup_timeout, async {
            if let Some(instance) = instance.as_ref() {
                if instance.id() != vm_id {
                    return Err(invalid(
                        "active VM handle differs from durable cleanup target",
                    ));
                }
                instance.destroy().await?;
            }
            // Missing process-local handles never establish authoritative absence.
            // Scoped confirmation also follows active-handle destruction.
            self.provider.cleanup_orphan_scoped(&scope, vm_id).await?;
            self.validate_cleanup_scope(&target)?;
            Ok::<(), OrchestratorError>(())
        })
        .await
        .map_err(|_| VmError::InvalidState("scoped VM cleanup deadline elapsed"))??;
        self.active.lock().await.remove(&run_id);
        let observation = checked(if instance.is_some() {
            RunCleanupVmObservation::destroyed(host(&scope)?, vm_id.clone())
        } else {
            RunCleanupVmObservation::authoritatively_absent(host(&scope)?, vm_id.clone())
        })?;
        let receipt = checked(RunCleanupReceipt::new(
            target,
            observation,
            OffsetDateTime::now_utc(),
        ))?;
        cleanup.repository.record_cleanup_receipt(&receipt).await?;
        Ok(receipt)
    }
}
