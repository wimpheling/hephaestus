use super::{
    Node, lock,
    models::{OciCleanupPhase, OciCleanupStatus},
    node::Leaf,
};
use std::sync::Arc;
impl Node {
    pub(super) async fn join_finished(self: &Arc<Self>) {
        let work = { lock(&self.work).clone() };
        if let Some(work) = work
            && work.ready()
        {
            let result = work.join(self).await.map_or_else(
                |_| Err(Arc::new(super::errors::failed("original operation task"))),
                std::convert::identity,
            );
            super::jobs::finish(self, result);
        }
        let watcher = { lock(&self.watcher).clone() };
        if let Some(watcher) = watcher
            && watcher.ready()
        {
            drop(watcher.join(self).await);
        }
        self.join_cleanup().await;
    }
    pub(super) async fn join_cleanup(self: &Arc<Self>) {
        let task = { lock(&self.cleanup).clone() };
        if let Some(task) = task
            && task.ready()
            && task.join(self).await.is_err()
        {
            let slot = lock(&self.cleanup);
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &task))
            {
                lock(&self.state).cleanup_error =
                    Some(Arc::new(super::errors::failed("original cleanup task")));
            }
        }
    }
    fn vm_cleanups_joined(&self) -> bool {
        lock(&self.state)
            .births
            .values()
            .all(|birth| birth.cleanup_history.iter().all(|task| task.joined()))
    }
    fn joined(&self) -> bool {
        self.vm_cleanups_joined()
            && lock(&self.work).as_ref().is_some_and(|task| task.joined())
            && lock(&self.watcher)
                .as_ref()
                .is_some_and(|task| task.joined())
            && lock(&self.cleanup)
                .as_ref()
                .is_none_or(|task| task.joined())
            && lock(&self.mkfs_cleanup).iter().all(|task| task.joined())
    }
    pub(super) fn delivery_physical(&self) -> bool {
        let stable = {
            let state = lock(&self.state);
            state.preparation.as_ref().is_none_or(|task| task.joined())
                && state.births.values().all(|birth| {
                    birth.detached
                        && birth
                            .instance
                            .as_ref()
                            .is_none_or(|instance| instance.id() == &birth.spec.id)
                })
                && state.outcome.is_some()
                && state.mkfs.as_ref().is_none_or(|mkfs| mkfs.parent_settled())
        };
        stable
            && self.vm_cleanups_joined()
            && lock(&self.cleanup)
                .as_ref()
                .is_some_and(|task| task.joined())
            && lock(&self.mkfs_cleanup).iter().all(|task| task.joined())
    }
    pub(super) fn delivery_joined(&self) -> bool {
        self.joined() && self.delivery_physical()
    }
    pub(super) fn status(&self) -> OciCleanupStatus {
        let joined = self.joined();
        let state = lock(&self.state);
        let retained_outputs = state.leaves[1..]
            .iter()
            .filter(|leaf| matches!(leaf, Leaf::Original(_)))
            .count();
        let preparation_joined = state.preparation.as_ref().is_none_or(|task| task.joined())
            && state.mkfs.as_ref().is_none_or(|mkfs| mkfs.parent_settled());
        let vm_held = state.births.values().any(|birth| {
            !birth.detached
                || birth
                    .instance
                    .as_ref()
                    .is_some_and(|instance| instance.id() != &birth.spec.id)
        });
        let scratch_held = match &state.leaves[0] {
            Leaf::Untouched => false,
            Leaf::Acquiring { .. } => true,
            Leaf::Original(pin) => !pin.removed(),
        };
        let unknown = state
            .leaves
            .iter()
            .any(|leaf| matches!(leaf, Leaf::Acquiring { .. }));
        let helper = state
            .mkfs
            .as_ref()
            .is_some_and(|mkfs| mkfs.helper_unqualified());
        let phase = if !joined || !preparation_joined || state.outcome.is_none() {
            OciCleanupPhase::Executing
        } else if vm_held {
            OciCleanupPhase::VmHeld
        } else if unknown {
            OciCleanupPhase::FilesystemHeld
        } else if scratch_held && helper {
            OciCleanupPhase::ScratchHelperUnqualified
        } else if scratch_held || state.cleanup_error.is_some() {
            OciCleanupPhase::FilesystemHeld
        } else {
            OciCleanupPhase::Settled
        };
        OciCleanupStatus {
            job_id: self.request.job_id,
            phase,
            retained_outputs: u8::try_from(retained_outputs).unwrap_or(u8::MAX),
            retained_errors: state.errors.len(),
        }
    }
    pub(super) fn recheck_outputs(&self) -> Result<(), super::OciWorkerError> {
        let pins = {
            lock(&self.state).leaves[1..]
                .iter()
                .filter_map(|leaf| {
                    if let Leaf::Original(pin) = leaf {
                        Some(pin.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        for pin in pins {
            pin.recheck()?;
        }
        Ok(())
    }
}
