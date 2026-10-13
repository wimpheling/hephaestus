//! Existing operation/task accounting and original generation keeper.
use super::{Custody, Registry};
use crate::provider::common::{Arc, ProviderLifetime, VmError};

impl Registry {
    pub(in crate::provider) async fn admit_start(
        self: &Arc<Self>,
        lifetime: &ProviderLifetime,
        birth: u64,
    ) -> Result<Operation, VmError> {
        let ids = lifetime.ids.lock().await;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return Err(VmError::InvalidState("VM provider admission is closed"));
        }
        let record = state
            .records
            .get_mut(&birth)
            .ok_or(VmError::InvalidState("VM birth is not registered"))?;
        if matches!(record.custody, Custody::Cleaned) {
            return Err(VmError::Destroyed);
        }
        record.snapshot.operations += 1;
        drop(state);
        drop(ids);
        Ok(Operation {
            registry: Arc::clone(self),
            birth,
            kind: Activity::Operation,
        })
    }

    pub(in crate::provider) fn lifecycle_task(
        self: &Arc<Self>,
        birth: u64,
    ) -> Result<Operation, VmError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let record = state
            .records
            .get_mut(&birth)
            .ok_or(VmError::InvalidState("VM birth is not registered"))?;
        record.lifecycle_tasks = record
            .lifecycle_tasks
            .checked_add(1)
            .ok_or(VmError::InvalidState("VM lifecycle task count exhausted"))?;
        drop(state);
        Ok(Operation {
            registry: Arc::clone(self),
            birth,
            kind: Activity::LifecycleTask,
        })
    }

    pub(in crate::provider) fn operation(self: &Arc<Self>, birth: u64) -> Operation {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&birth) {
            record.snapshot.operations += 1;
        }
        drop(state);
        Operation {
            registry: Arc::clone(self),
            birth,
            kind: Activity::Operation,
        }
    }
}

// Activity bookkeeping, not another operation lock or cleanup evidence.
// LifecycleTask additionally covers tasks queued before the operation mutex.
enum Activity {
    Operation,
    LifecycleTask,
}
pub(in crate::provider) struct Operation {
    registry: Arc<Registry>,
    birth: u64,
    kind: Activity,
}
impl Drop for Operation {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(record) = state.records.get_mut(&self.birth) {
            match self.kind {
                Activity::Operation => record.snapshot.operations -= 1,
                Activity::LifecycleTask => record.lifecycle_tasks -= 1,
            }
        }
        drop(state);
        self.registry.changed.notify_waiters();
    }
}

pub(super) async fn keep_generation(registry: Arc<Registry>, lifetime: Arc<ProviderLifetime>) {
    loop {
        let changed = registry.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let complete = {
            let ids = lifetime.ids.lock().await;
            let mut state = registry
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let complete = state.settled();
            if complete {
                state.keeper = false;
            }
            drop(state);
            drop(ids);
            complete
        };
        if complete {
            return;
        }
        changed.await;
    }
}
