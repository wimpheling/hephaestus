//! One registry per supervised owner; waiters retain their exact mutex identity.

use runtime_types::RunId;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};

#[derive(Default)]
pub struct RunOperationGuards {
    operations: Mutex<HashMap<RunId, Weak<AsyncMutex<()>>>>,
    starts: Mutex<HashMap<RunId, Weak<()>>>,
}

impl RunOperationGuards {
    fn operation(&self, run: RunId) -> Result<Arc<AsyncMutex<()>>, OrchestratorError> {
        let mut registry = self
            .operations
            .lock()
            .map_err(|_| invalid("run operation registry is poisoned"))?;
        registry.retain(|_, entry| entry.strong_count() > 0);
        if let Some(guard) = registry.get(&run).and_then(Weak::upgrade) {
            return Ok(guard);
        }
        let guard = Arc::new(AsyncMutex::new(()));
        registry.insert(run, Arc::downgrade(&guard));
        drop(registry);
        Ok(guard)
    }

    pub fn claim_start(&self, run: RunId) -> Result<Arc<()>, OrchestratorError> {
        let mut registry = self
            .starts
            .lock()
            .map_err(|_| invalid("run start registry is poisoned"))?;
        registry.retain(|_, entry| entry.strong_count() > 0);
        if registry.get(&run).and_then(Weak::upgrade).is_some() {
            return Err(OrchestratorError::RunInProgress(run));
        }
        let claim = Arc::new(());
        registry.insert(run, Arc::downgrade(&claim));
        drop(registry);
        Ok(claim)
    }
}

impl RunOrchestrator {
    pub(super) async fn lock_run_operation(
        &self,
        run: RunId,
    ) -> Result<Option<OwnedMutexGuard<()>>, OrchestratorError> {
        if self.canonical_cleanup.is_none() {
            return Ok(None);
        }
        Ok(Some(
            self.operation_guards.operation(run)?.lock_owned().await,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_waiters_keep_one_mutex_while_unrelated_runs_are_parallel() {
        let registry = RunOperationGuards::default();
        let run = RunId::new();
        let first = registry.operation(run).unwrap();
        let held = first.clone().lock_owned().await;
        let waiter = registry.operation(run).unwrap();
        drop(first);
        assert!(Arc::ptr_eq(&waiter, &registry.operation(run).unwrap()));
        let mut queued = tokio::spawn(waiter.clone().lock_owned());
        let other = registry.operation(RunId::new()).unwrap();
        assert!(other.try_lock().is_ok());
        assert!(!queued.is_finished());
        drop(held);
        let acquired = (&mut queued).await.unwrap();
        drop(queued);
        assert!(Arc::ptr_eq(&waiter, &registry.operation(run).unwrap()));
        drop(acquired);
        let weak = Arc::downgrade(&waiter);
        drop(waiter);
        assert!(weak.upgrade().is_none());
        let recreated = registry.operation(run).unwrap();
        assert!(recreated.try_lock().is_ok());
    }

    #[test]
    fn live_start_claim_rejects_duplicate_and_reopens_only_after_token_drop() {
        let registry = RunOperationGuards::default();
        let run = RunId::new();
        let claim = registry.claim_start(run).unwrap();
        assert!(
            matches!(registry.claim_start(run), Err(OrchestratorError::RunInProgress(id)) if id == run)
        );
        assert!(registry.claim_start(RunId::new()).is_ok());
        drop(claim);
        assert!(registry.claim_start(run).is_ok());
    }
}
