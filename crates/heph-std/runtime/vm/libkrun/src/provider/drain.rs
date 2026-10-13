//! Cleanup of exact admitted births in this live generation, not global absence.
use super::common::{Arc, HashSet, Ordering, ProviderInner, VmError, VmInstance, timeout};
use super::{helpers::unavailable_error, launch::LaunchState, ownership::OwnerGuard};

pub(super) async fn run(provider: Arc<ProviderInner>, guard: OwnerGuard) -> Result<(), VmError> {
    let serial = provider.registry.drain.lock().await;
    let outcome = drain_known(&provider, &guard).await;
    drop(serial);
    drop(guard);
    drop(provider);
    outcome
}

async fn drain_known(provider: &ProviderInner, guard: &OwnerGuard) -> Result<(), VmError> {
    let owner = provider
        .owner
        .get()
        .ok_or(VmError::InvalidState("generation drain owner missing"))?;
    let ids = provider.ids.lock().await;
    owner.validate_guard(&provider.config, guard)?;
    provider.registry.close();
    let mut pending = {
        let entries = provider
            .pending_launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Retry eligibility belongs to this snapshot, before any cancellation
        // can produce a new Held failure while earlier births are awaited.
        let pending = entries
            .values()
            .map(|entry| {
                let retry_held = matches!(*entry.state.borrow(), LaunchState::Held(_));
                (Arc::clone(entry), retry_held)
            })
            .collect::<Vec<_>>();
        // Every pending owner is canceled before waiting for any one of them.
        // Provision acknowledgement uses this same IDs lock and checks the flag.
        for (entry, _) in &pending {
            entry.request_drain();
        }
        pending
    };
    pending.sort_by_key(|(entry, _)| entry.birth.load(Ordering::Acquire));
    let pending_births: HashSet<_> = pending
        .iter()
        .map(|(entry, _)| entry.birth.load(Ordering::Acquire))
        .collect();
    let instances = provider
        .registry
        .instances()
        .into_iter()
        // During handoff the original launch task owns instance cleanup. Do not
        // silently retry its first failure again within the same drain call.
        .filter(|instance| !pending_births.contains(&instance.birth))
        .collect::<Vec<_>>();
    drop(ids);

    let budget = provider.config.startup_timeout;
    let mut failure = None;
    for (entry, retry_held) in pending {
        let result = timeout(budget, entry.drain_cleanup(retry_held))
            .await
            .unwrap_or_else(|_| {
                Err(unavailable_error(
                    "generation drain",
                    "pending launch cleanup remains unconfirmed",
                ))
            });
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
    }
    for instance in instances {
        // Destroy transfers execution to the actual instance's owning task. A
        // deadline drops only its waiter, never that task or original custody.
        let result = timeout(budget, instance.destroy())
            .await
            .unwrap_or_else(|_| {
                Err(unavailable_error(
                    "generation drain",
                    "instance cleanup remains unconfirmed",
                ))
            });
        if let Err(error) = result {
            failure.get_or_insert(error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    timeout(budget, provider.registry.wait_settled())
        .await
        .map_err(|_| {
            unavailable_error(
                "generation drain",
                "known launch or lifecycle tasks remain active",
            )
        })?;
    owner.validate_guard(&provider.config, guard)
}
