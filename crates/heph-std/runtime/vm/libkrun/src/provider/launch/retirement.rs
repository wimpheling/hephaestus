//! Existing exact cleanup, retirement and retained retry loop.
use super::{LaunchFailure, LaunchHandoff, LaunchState, PendingLaunch, RetryInbox};
use crate::provider::{
    common::{Arc, LibkrunInstance, ProviderInner, VmError, oneshot},
    helpers::provider_error,
    ownership::OwnerGuard,
};

async fn cleanup_entry(
    provider: &ProviderInner,
    entry: &PendingLaunch,
    instance: Option<&Arc<LibkrunInstance>>,
) -> Result<(), VmError> {
    if let Some(instance) = instance {
        vm_trait::VmInstance::destroy(instance.as_ref()).await
    } else {
        entry.cleanup(provider).await
    }
}

pub(super) async fn retire(provider: &ProviderInner, entry: &PendingLaunch, release_id: bool) {
    let mut ids = provider.ids.lock().await;
    // No await between releasing the exact reservation and retiring its entry.
    let mut pending = provider
        .pending_launches
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pending
        .get(&entry.id)
        .is_some_and(|current| std::ptr::eq(current.as_ref(), entry))
    {
        if release_id {
            ids.remove(&entry.id);
        }
        pending.remove(&entry.id);
    }
    drop(pending);
    let pid = entry
        .custody
        .process()
        .ok()
        .flatten()
        .and_then(|process| process.pid);
    provider.registry.launch_finished(
        entry.birth.load(super::common::Ordering::Acquire),
        release_id,
        pid,
    );
    drop(ids);
}

pub(super) async fn hold_or_retire(
    provider: Arc<ProviderInner>,
    entry: Arc<PendingLaunch>,
    guard: Option<OwnerGuard>,
    instance: Option<Arc<LibkrunInstance>>,
    cause: VmError,
    result: Option<oneshot::Sender<Result<LaunchHandoff, VmError>>>,
    mut retries: RetryInbox,
) {
    let cleanup = cleanup_entry(&provider, &entry, instance.as_ref()).await;
    let cause = match cleanup {
        Ok(()) => {
            retire(&provider, &entry, true).await;
            entry.state.send_replace(LaunchState::Cleaned);
            if let Some(result) = result {
                let _sent = result.send(Err(cause));
            }
            return;
        }
        Err(cleanup) => {
            let cause = Arc::new(cause);
            let failure = Arc::new(LaunchFailure {
                cause: Arc::clone(&cause),
                cleanup: Arc::new(cleanup),
            });
            entry
                .state
                .send_replace(LaunchState::Held(Arc::clone(&failure)));
            if let Some(result) = result {
                let _sent = result.send(Err(provider_error("worker-launch-cleanup", failure)));
            }
            cause
        }
    };
    // The task retains original provider/owner custody during uncertainty. The
    // map entry has no Arc back-reference and no public abort handle.
    while let Some(reply) = retries.recv().await {
        entry.state.send_replace(LaunchState::Working);
        match cleanup_entry(&provider, &entry, instance.as_ref()).await {
            Ok(()) => {
                retire(&provider, &entry, true).await;
                entry.state.send_replace(LaunchState::Cleaned);
                let _sent = reply.send(Ok(()));
                drop(guard);
                return;
            }
            Err(cleanup) => {
                let failure = Arc::new(LaunchFailure {
                    cause: Arc::clone(&cause),
                    cleanup: Arc::new(cleanup),
                });
                entry
                    .state
                    .send_replace(LaunchState::Held(Arc::clone(&failure)));
                let _sent = reply.send(Err(provider_error("worker-launch-cleanup", failure)));
            }
        }
    }
}
