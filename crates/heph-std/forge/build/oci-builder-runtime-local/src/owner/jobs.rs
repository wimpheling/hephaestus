use super::{Node, OciWorkerError, lock, models::Outcome, task::Task};
use std::sync::Arc;
pub(super) async fn execute(
    operation: crate::VmOciOperation,
    request: oci_builder_worker::IsolatedOciBuild,
) -> Result<crate::VerifiedVmOciOutput, OciWorkerError> {
    let paths = [
        operation
            .scratch_root
            .join(format!("{}.raw", request.job_id)),
        operation.candidate_root.join(request.job_id.to_string()),
        operation.verification_root.join(request.job_id.to_string()),
    ];
    let (node, new, matched) = operation.custody.registry.admit(request, paths)?;
    let custody = operation.custody.clone();
    if new {
        start(&node, operation);
    } else {
        node.join_finished().await;
    }
    // Poll actual joins even if an unexpected outer watcher failure occurred.
    // Every response (including a conflicting retry) is fenced against unsafe
    // existing caller checkout cleanup while this SAME original VM is held.
    let outcome = loop {
        node.join_finished().await;
        let known = { lock(&node.state).outcome.clone() };
        if known.is_some() && lock(&node.cleanup).is_none() {
            node.kick(custody.clone());
        }
        if node.delivery_joined() {
            if let Some(outcome) = known {
                break outcome;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    if !matched {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    outcome.map_err(super::errors::observed)
}
pub(super) fn start(node: &Arc<Node>, operation: crate::VmOciOperation) {
    let custody = operation.custody.clone();
    let owner = node.clone();
    start_work(node, custody, async move {
        super::guest::work(&operation, &owner)
            .await
            .map_err(Arc::new)
    });
}
pub(super) fn start_work(
    node: &Arc<Node>,
    custody: Arc<super::Owner>,
    work: impl std::future::Future<Output = Outcome> + Send + 'static,
) {
    let (release, registered) = tokio::sync::oneshot::channel();
    let owner = node.clone();
    let work = Task::new(tokio::spawn(async move {
        if let Err(error) = registered.await {
            owner.error(Arc::new(error));
            return Err(Arc::new(OciWorkerError::InvalidConfiguration));
        }
        work.await
    }));
    *lock(&node.work) = Some(work.clone());
    let owner = node.clone();
    let watcher = Task::new(tokio::spawn(async move {
        let result = work.join(&owner).await.map_or_else(
            |_| Err(Arc::new(super::errors::failed("original operation task"))),
            std::convert::identity,
        );
        finish(&owner, result);
        owner.kick(custody);
        // This owning delivery watcher waits only producer/preparation/cleanup,
        // never itself. Existing worker must not retire its mounted checkout
        // merely because a failed producer result became available.
        loop {
            owner.join_cleanup().await;
            if owner.delivery_physical() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }));
    *lock(&node.watcher) = Some(watcher);
    let _registered = release.send(());
}
pub(super) fn finish(node: &Arc<Node>, outcome: Outcome) {
    let mut state = lock(&node.state);
    if state.outcome.is_some() {
        return;
    }
    if let Err(error) = &outcome {
        state.errors.push(error.clone());
    }
    state.outcome = Some(outcome);
    drop(state);
    node.changed.notify_waiters();
}
