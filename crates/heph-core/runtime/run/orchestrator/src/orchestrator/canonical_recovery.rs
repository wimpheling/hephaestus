//! Restart cleanup groups complete canonical snapshots by run, never by lease.

use run_domain::{RunKind, RunState};
use runtime_types::RunId;
use std::collections::BTreeSet;
use time::OffsetDateTime;

use super::{OrchestratorError, RunOrchestrator, SupervisorReconciliationCause};

impl RunOrchestrator {
    pub(super) async fn stale_cleanup_runs(&self) -> Result<BTreeSet<RunId>, OrchestratorError> {
        if self.invocation_scope.is_some() {
            return self
                .invocation_recovery_inventory(Some(OffsetDateTime::now_utc()))
                .await;
        }
        self.provider.owner_scope()?;
        Ok(self
            .volumes
            .stale_leases(OffsetDateTime::now_utc())
            .await?
            .into_iter()
            .map(|lease| lease.run_id)
            .collect())
    }

    pub(super) async fn recover_canonical_runs(
        &self,
        run_ids: BTreeSet<RunId>,
    ) -> Result<usize, OrchestratorError> {
        self.reconcile_canonical_runs(run_ids, SupervisorReconciliationCause::Restart)
            .await
    }

    pub(super) async fn reconcile_canonical_runs(
        &self,
        run_ids: BTreeSet<RunId>,
        cause: SupervisorReconciliationCause,
    ) -> Result<usize, OrchestratorError> {
        let count = run_ids.len();
        for run_id in run_ids {
            if self.invocation_scope.is_some() {
                self.recover_invocation_run(run_id, cause).await?;
            } else {
                let run = self.repository.get(run_id).await?;
                self.finish_reconciled_run(run, cause).await?;
            }
        }
        Ok(count)
    }

    pub(super) async fn restart_canonical_cleanup(&self) -> Result<usize, OrchestratorError> {
        if self.invocation_scope.is_some() {
            return self
                .recover_canonical_runs(self.invocation_recovery_inventory(None).await?)
                .await;
        }
        let mut run_ids = self.stale_cleanup_runs().await?;
        run_ids.extend(
            self.repository
                .recoverable_runs()
                .await?
                .into_iter()
                .map(|run| run.id),
        );
        let mut recovered = self.recover_canonical_runs(run_ids).await?;
        // Global transient sweeps must follow confirmed scoped guest cleanup.
        // Any unresolved historical run stops here while its fences remain held.
        recovered += self.workspaces.recover().await?;
        recovered += self.runtimes.recover().await?;
        recovered += self.secrets.recover().await?;
        recovered += self.authority.recover().await?;
        recovered += self.completion.recover().await?;
        recovered += self.runtime_git_workspace.recover_runtime_git().await?;
        Ok(recovered)
    }

    pub(super) async fn invocation_recovery_inventory(
        &self,
        stale_before: Option<OffsetDateTime>,
    ) -> Result<BTreeSet<RunId>, OrchestratorError> {
        let scope = self.provider.owner_scope()?;
        if self.invocation_scope.as_ref() != Some(&scope) || self.legacy_scope.is_some() {
            return Err(super::canonical_cleanup::invalid(
                "Invocation recovery owner differs",
            ));
        }
        let cleanup = self.canonical_cleanup.as_ref().ok_or_else(|| {
            super::canonical_cleanup::invalid("Invocation recovery cleanup is absent")
        })?;
        let ids = cleanup
            .repository
            .qualified_invocation_recovery_inventory(&scope, stale_before)
            .await?;
        let unique: BTreeSet<_> = ids.iter().copied().collect();
        if ids.len() != unique.len() || ids.iter().any(|id| id.as_uuid().is_nil()) {
            return Err(super::canonical_cleanup::invalid(
                "Invocation recovery inventory IDs differ",
            ));
        }
        Ok(unique)
    }

    async fn recover_invocation_run(
        &self,
        run_id: RunId,
        cause: SupervisorReconciliationCause,
    ) -> Result<(), OrchestratorError> {
        // Hold the original live-Start exclusion through the complete cleanup.
        // Canonical physical cleanup reacquires the existing operation mutex.
        let claim = self.operation_guards.claim_start(run_id)?;
        let operation = self.lock_run_operation(run_id).await?;
        let scope = self.invocation_cancel_owner()?;
        let prepared = self.prepare_invocation_cancel(run_id, &scope).await?;
        let run = match &prepared {
            Some(prepared) => prepared.run.clone(),
            None => self.repository.get(run_id).await?,
        };
        if run.kind != RunKind::Invocation
            || run.attachment_id.is_some()
            || run.vm_id.as_deref() != Some(run_id.to_string().as_str())
        {
            return Err(super::canonical_cleanup::invalid(
                "Invocation recovery original differs",
            ));
        }
        if run.state == RunState::Queued && run.cancel_requested_at.is_some() {
            self.repository
                .transition(run_id, RunState::Cancelled, None, None)
                .await?;
        }
        // Existing cleanup reacquires this exact operation mutex. Permanent
        // closure and the retained live-Start claim exclude new IO in the gap.
        drop(operation);
        if let Some(prepared) = prepared {
            self.canonical_cleanup_expected(run_id, None, Some(&prepared.target))
                .await?;
        } else {
            self.finish_reconciled_run(self.repository.get(run_id).await?, cause)
                .await?;
        }
        drop(claim);
        Ok(())
    }
}
