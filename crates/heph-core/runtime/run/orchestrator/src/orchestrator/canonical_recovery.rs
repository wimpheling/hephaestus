//! Restart cleanup groups complete canonical snapshots by run, never by lease.

use runtime_types::RunId;
use std::collections::BTreeSet;
use time::OffsetDateTime;

use super::{OrchestratorError, RunOrchestrator};

impl RunOrchestrator {
    pub(super) async fn stale_cleanup_runs(&self) -> Result<BTreeSet<RunId>, OrchestratorError> {
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
        let count = run_ids.len();
        for run_id in run_ids {
            let run = self.repository.get(run_id).await?;
            self.finish_recovered_run(run).await?;
        }
        Ok(count)
    }

    pub(super) async fn restart_canonical_cleanup(&self) -> Result<usize, OrchestratorError> {
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
}
