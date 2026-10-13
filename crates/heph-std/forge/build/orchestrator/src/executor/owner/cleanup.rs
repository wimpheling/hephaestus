use super::{
    BuildCleanupError, BuildCleanupPhase, BuildCleanupStatus, BuildExecutionError, BuildExecutor,
    BuildRequestId, lock, node::Node,
};
use std::{sync::Arc, time::Duration};

impl BuildExecutor {
    /// Monotonically closes new Execute/Retry/Verify intake for this live owner.
    /// Admitted jobs retain custody; genuine saved finalization can continue.
    pub fn close_admission(&self) {
        self.registry.close();
    }
    /// Comparison-only inventory; no prior-process or hostwide absence proof.
    #[must_use]
    pub fn cleanup_inventory(&self) -> Vec<BuildCleanupStatus> {
        let mut rows = self
            .registry
            .nodes()
            .iter()
            .map(|node| node.status())
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| row.build_request_id);
        rows
    }
    /// Initiates original cleanup before observing job settlement. Budget
    /// expiry/waiter abandonment never aborts the retained execution/VM jobs.
    /// # Errors
    /// Returns held observations until every original live owner is settled.
    pub async fn close_and_cleanup(&self, budget: Duration) -> Result<(), BuildCleanupError> {
        self.close_admission();
        let nodes = self.registry.nodes();
        for node in &nodes {
            node.join_finished_cleanup().await;
            node.kick_cleanup(true);
        }
        self.finish_observation(observe(&nodes, budget).await)
    }
    /// Retries cleanup of this registered original only, never name lookup.
    /// # Errors
    /// Unknown identity or unresolved original cleanup is held.
    pub async fn retry_cleanup(
        &self,
        id: BuildRequestId,
        budget: Duration,
    ) -> Result<(), BuildCleanupError> {
        let Some(node) = self.registry.get(id) else {
            return Err(BuildCleanupError {
                remaining: vec![BuildCleanupStatus {
                    build_request_id: id,
                    phase: BuildCleanupPhase::WorkspaceHeld,
                    retained_errors: 0,
                }],
                root_held: false,
            });
        };
        node.close();
        node.join_finished_cleanup().await;
        node.kick_cleanup(true);
        self.finish_observation(observe(&[node], budget).await)
    }
    fn finish_observation(
        &self,
        result: Result<(), BuildCleanupError>,
    ) -> Result<(), BuildCleanupError> {
        if let Err(error) = self.workspace_root.recheck() {
            self.registry.root_error(Arc::new(error));
            return Err(BuildCleanupError {
                remaining: self.cleanup_inventory(),
                root_held: true,
            });
        }
        result
    }
}
pub(super) async fn observe(
    nodes: &[Arc<Node>],
    budget: Duration,
) -> Result<(), BuildCleanupError> {
    let result = tokio::time::timeout(budget, async {
        loop {
            for node in nodes {
                node.join_finished_jobs().await;
            }
            if nodes.iter().all(|node| node.settled()) {
                return;
            }
            // Observation only: no tasks are spawned/retried by this poll.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if result.is_ok() {
        Ok(())
    } else {
        Err(BuildCleanupError {
            remaining: nodes
                .iter()
                .filter(|node| !node.settled())
                .map(|node| node.status())
                .collect(),
            root_held: false,
        })
    }
}
impl Node {
    pub(super) fn kick_cleanup(self: &Arc<Self>, retry: bool) {
        if !retry && lock(&self.state).cleanup_failure.is_some() {
            return;
        }
        let mut job = lock(&self.cleanup_job);
        if job.as_ref().is_some_and(|job| !job.joined()) {
            return;
        }
        let owner = self.clone();
        let (release, registered) = tokio::sync::oneshot::channel();
        *job = Some(super::task::Task::new(tokio::spawn(async move {
            if let Err(error) = registered.await {
                owner.error(Arc::new(error));
                lock(&owner.state).cleanup_error = Some(Arc::new(BuildExecutionError::WorkerJoin));
                return;
            }
            if let Err(error) = owner.cleanup_original().await {
                let error = Arc::new(error);
                owner.error(error.clone());
                lock(&owner.state).cleanup_error = Some(error);
            } else {
                lock(&owner.state).cleanup_error = None;
            }
            owner.changed.notify_waiters();
        })));
        // Retain the actual handle before this job can poll cleanup IO.
        let _released = release.send(());
        drop(job);
    }
    async fn cleanup_original(self: &Arc<Self>) -> Result<(), BuildExecutionError> {
        // VM is cleaned concurrently with a DB/monitor-stalled execution. Do
        // NOT wait for execution first; newly installed token wakes this loop.
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let ready = {
                let state = lock(&self.state);
                state.attempt.is_some()
                    || state.outcome.is_some()
                    || !state.git_children.is_empty()
                    || state.preparation.is_some()
            };
            if ready {
                break;
            }
            changed.await;
        }
        self.detach().await?;
        self.stop_git().await?;
        let preparation = lock(&self.state).preparation.clone();
        if let Some(preparation) = preparation {
            let _result = preparation.join().await;
        }
        // Positive detach does NOT authorize deleting a workspace still in use
        // by preparation/import/finalization, including blocked DB operations.
        self.outcome().await;
        self.detach().await?;
        self.remove_workspace()
    }
}
