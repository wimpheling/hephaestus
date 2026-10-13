//! Managed per-Run worker reconciliation; neither cancellation nor absence proof.
use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};

/// The actual supervisor event that caused trusted per-Run reconciliation.
///
/// This supplies no actor, cancellation, physical receipt or dispatch authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorReconciliationCause {
    /// Durable reconciliation following a supervisor restart.
    Restart,
    /// Durable reconciliation while the retained supervisor is stopping.
    Stop,
}

impl SupervisorReconciliationCause {
    pub(super) const fn failure(self) -> &'static str {
        match self {
            Self::Restart => "supervisor restarted while run resources were active",
            Self::Stop => "supervisor stopped while run resources were active",
        }
    }
    pub(super) const fn legacy_failure(self) -> &'static str {
        match self {
            Self::Restart => "supervisor restarted while resources were active",
            Self::Stop => "supervisor stopped while run resources were active",
        }
    }
}

impl RunOrchestrator {
    /// Reconciles the complete qualified managed family's actionable originals.
    ///
    /// Requires exactly one qualified Invocation or strict Legacy composition.
    /// Reuses per-Run scoped cleanup, preserving existing terminal and cancellation
    /// evidence. Positively qualified open no-IO work remains redeliverable.
    /// No global transient recovery callbacks are invoked. Success is not a
    /// Ready/Stopped, host-absence, instance-Drain or permanent-closure certificate;
    /// callers still require complete pre/post observations and retained owners.
    ///
    /// # Errors
    /// Returns an error for unsupported/conflicting composition, owner/inventory
    /// uncertainty, a live Start claim, or any unconfirmed per-Run cleanup.
    pub async fn reconcile_managed_runs(
        &self,
        cause: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        match (&self.invocation_scope, &self.legacy_scope) {
            (Some(_), None) if self.canonical_cleanup.is_some() && self.plural_volumes => {
                let runs = self.invocation_recovery_inventory(None).await?;
                self.reconcile_canonical_runs(runs, cause).await?;
                Ok(())
            }
            (None, Some(_)) => self.reconcile_legacy_runs(cause).await,
            _ => Err(invalid(
                "managed reconciliation composition is absent or conflicting",
            )),
        }
    }
}
