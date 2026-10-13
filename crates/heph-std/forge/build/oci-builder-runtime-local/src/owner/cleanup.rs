use super::{
    Node, OciWorkerError, Owner, lock,
    models::{OciCleanupHeld, OciCleanupPhase, OciCleanupStatus},
    node::Leaf,
    task::Task,
};
use std::{sync::Arc, time::Duration};
impl crate::VmOciOperation {
    /// Monotonically closes this original operation's intake across all clones.
    pub fn close_admission(&self) {
        self.custody.close();
    }
    /// Initiates same-attempt cleanup before observing original task settlement.
    /// Verified/uncertain candidate and verifier output remain pinned/retained.
    /// # Errors
    /// Returns Held on unresolved original tasks, roots, VM or formatter helpers.
    pub async fn close_and_cleanup(&self, budget: Duration) -> Result<(), OciCleanupHeld> {
        self.close_admission();
        let nodes = self.custody.registry.nodes();
        for node in &nodes {
            node.join_cleanup().await;
            node.kick(self.custody.clone());
        }
        observe(&self.custody, &nodes, budget).await
    }
    /// Retries only this registered original; never reconstructs it by name.
    /// # Errors
    /// Unknown original or unresolved cleanup returns Held without losing custody.
    pub async fn retry_cleanup(
        &self,
        job_id: uuid::Uuid,
        budget: Duration,
    ) -> Result<(), OciCleanupHeld> {
        let Some(node) = self.custody.registry.get(job_id) else {
            return Err(OciCleanupHeld {
                remaining: vec![OciCleanupStatus {
                    job_id,
                    phase: OciCleanupPhase::FilesystemHeld,
                    retained_outputs: 0,
                    retained_errors: 0,
                }],
                roots_held: false,
            });
        };
        node.close();
        node.join_cleanup().await;
        node.kick(self.custody.clone());
        observe(&self.custody, &[node], budget).await
    }
    /// Comparison-only live inventory; no publication/hostwide stop certificate.
    #[must_use]
    pub fn cleanup_inventory(&self) -> Vec<OciCleanupStatus> {
        self.custody
            .registry
            .nodes()
            .iter()
            .map(|node| node.status())
            .collect()
    }
}
pub(super) async fn observe(
    owner: &Owner,
    nodes: &[Arc<Node>],
    budget: Duration,
) -> Result<(), OciCleanupHeld> {
    let result = tokio::time::timeout(budget, async {
        loop {
            for node in nodes {
                node.join_finished().await;
            }
            if nodes
                .iter()
                .all(|node| node.status().phase == OciCleanupPhase::Settled)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let mut roots_held = false;
    if let Err(error) = owner.roots.recheck() {
        lock(&owner.errors).push(Arc::new(error));
        roots_held = true;
    }
    for node in nodes {
        if let Err(error) = node.recheck_outputs() {
            node.error(Arc::new(error));
            roots_held = true;
        }
    }
    if result.is_ok() && !roots_held {
        Ok(())
    } else {
        Err(OciCleanupHeld {
            remaining: nodes
                .iter()
                .filter(|node| roots_held || node.status().phase != OciCleanupPhase::Settled)
                .map(|node| node.status())
                .collect(),
            roots_held,
        })
    }
}
impl Node {
    pub(super) fn kick(self: &Arc<Self>, owner: Arc<Owner>) {
        let mut slot = lock(&self.cleanup);
        if slot.as_ref().is_some_and(|task| !task.joined()) {
            return;
        }
        let node = self.clone();
        let (release, registered) = tokio::sync::oneshot::channel();
        *slot = Some(Task::new(tokio::spawn(async move {
            if let Err(error) = registered.await {
                node.error(Arc::new(error));
                lock(&node.state).cleanup_error =
                    Some(Arc::new(OciWorkerError::InvalidConfiguration));
                return;
            }
            let result = node.clean_original(&owner).await;
            let mut state = lock(&node.state);
            state.cleanup_error = match result {
                Ok(()) => None,
                Err(error) => {
                    let error = Arc::new(error);
                    state.errors.push(error.clone());
                    Some(error)
                }
            };
            drop(state);
            node.changed.notify_waiters();
        })));
        let _registered = release.send(());
        drop(slot);
    }
    async fn clean_original(self: &Arc<Self>, owner: &Owner) -> Result<(), OciWorkerError> {
        // Close forbids any later token/formatter admission. Cleanup starts
        // before waiting for original preparation/work, including stalled IO.
        self.close();
        let attempts = self.start_detach();
        let formatter = self.start_stop_mkfs();
        // Both VM requests and formatter cleanup own registered handles now.
        // A stalled/failing first resource cannot prevent another from starting.
        let mut failure = self.join_detach(attempts).await.err();
        if let Some(formatter) = formatter
            && let Err(error) = self.join_formatter(formatter).await
        {
            failure.get_or_insert(error);
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let preparation = { lock(&self.state).preparation.clone() };
        if let Some(preparation) = preparation {
            drop(preparation.join(self).await);
        }
        self.detach().await?;
        let work = { lock(&self.work).clone() };
        if let Some(work) = work {
            let outcome = work.join(self).await.map_or_else(
                |_| Err(Arc::new(super::errors::failed("original operation task"))),
                std::convert::identity,
            );
            super::jobs::finish(self, outcome);
        }
        self.detach().await?;
        self.require_detached()?;
        owner.roots.recheck()?;
        self.recheck_outputs()?;
        let scratch = {
            let state = lock(&self.state);
            match &state.leaves[0] {
                Leaf::Untouched => None,
                Leaf::Acquiring { .. } => return Err(OciWorkerError::UnsafeMaterializationPath),
                Leaf::Original(pin) => Some(pin.clone()),
            }
        };
        if let Some(scratch) = scratch {
            if lock(&self.state)
                .mkfs
                .as_ref()
                .is_some_and(|mkfs| mkfs.helper_unqualified())
            {
                // Real original parent wait is insufficient for arbitrary
                // configured helpers. No pathname-based builtin exemption.
                return Err(OciWorkerError::InvalidConfiguration);
            }
            scratch.remove_scratch()?;
        }
        Ok(())
    }
}
