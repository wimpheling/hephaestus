//! Actual work/watcher/cleanup joins and retained failure reconciliation.
use super::{
    BuildExecutionError, jobs, lock,
    models::{Mode, Outcome},
    node::Node,
};
use std::sync::Arc;
impl Node {
    pub(super) fn jobs_settled(&self) -> bool {
        lock(&self.work_job)
            .as_ref()
            .is_some_and(|job| job.joined())
            && lock(&self.execution_job)
                .as_ref()
                .is_some_and(|job| job.joined())
            && lock(&self.cleanup_job)
                .as_ref()
                .is_none_or(|job| job.joined())
            && lock(&self.git_cleanup).iter().all(|job| job.settled())
    }
    pub(super) async fn join_finished_jobs(self: &Arc<Self>) {
        let work = { lock(&self.work_job).clone() };
        if let Some(work) = work
            && work.ready()
        {
            let outcome = work.join(self).await.map_or_else(
                |_| {
                    let error = Arc::new(BuildExecutionError::WorkerJoin);
                    if lock(&self.state).mode == Mode::Verify {
                        Outcome::Verify(Err(error))
                    } else {
                        Outcome::Build(Err(error))
                    }
                },
                std::convert::identity,
            );
            jobs::finish(self, &work, outcome);
        }
        let execution = { lock(&self.execution_job).clone() };
        if let Some(execution) = execution
            && execution.ready()
        {
            // Its raw failure is historical. The independently retained actual
            // work still must be joined and physical cleanup positively settled.
            drop(execution.join(self).await);
        }
        self.join_finished_cleanup().await;
    }
    pub(super) async fn join_finished_cleanup(self: &Arc<Self>) {
        let cleanup = { lock(&self.cleanup_job).clone() };
        if let Some(cleanup) = cleanup
            && cleanup.ready()
            && cleanup.join(self).await.is_err()
        {
            // Never overwrite a newer pass with a stale prior join failure.
            let slot = lock(&self.cleanup_job);
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &cleanup))
            {
                lock(&self.state).cleanup_error = Some(Arc::new(BuildExecutionError::WorkerJoin));
            }
        }
    }
}
