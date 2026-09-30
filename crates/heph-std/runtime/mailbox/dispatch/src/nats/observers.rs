use super::contract::MailboxDispatchStore;
use async_trait::async_trait;
use run_domain::Run;
use run_orchestrator::{
    RunCompletionError, RunCompletionObserver, RunResourceObservationError, RunResourceObserver,
};
use std::sync::Arc;

/// Observes cleaned VM runs and settles their delivery from durable run facts.
#[derive(Clone)]
pub struct MailboxRunCompletion {
    store: Arc<dyn MailboxDispatchStore>,
}

impl MailboxRunCompletion {
    /// Creates a mailbox completion observer.
    #[must_use]
    pub fn new(store: Arc<dyn MailboxDispatchStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl RunCompletionObserver for MailboxRunCompletion {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        self.store
            .settle_run(run)
            .await
            .map_err(|error| RunCompletionError::redacted(error.to_string()))
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        self.store
            .recover()
            .await
            .map_err(|error| RunCompletionError::redacted(error.to_string()))
    }
}

/// Records mailbox-attempt resource evidence at the VM pre-provision boundary.
#[derive(Clone)]
pub struct MailboxRunResources {
    store: Arc<dyn MailboxDispatchStore>,
}

impl MailboxRunResources {
    /// Creates the pre-provisioning mailbox resource observer.
    #[must_use]
    pub fn new(store: Arc<dyn MailboxDispatchStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl RunResourceObserver for MailboxRunResources {
    async fn record(&self, run: &Run) -> Result<(), RunResourceObservationError> {
        self.store
            .record_run_resources(run)
            .await
            .map_err(|error| RunResourceObservationError::redacted(error.to_string()))
    }
}
