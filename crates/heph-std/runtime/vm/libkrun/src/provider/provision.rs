//! Unchanged native provision/handoff, plus optional original token binding.
use super::LibkrunProvider;
use crate::provider::common::{
    Arc, PendingLaunch, VmError, VmInstance, VmSpec, oneshot, prepare_spec,
};
use crate::provider::helpers::unavailable_error;
use crate::provider::{launch, ownership};

pub(super) async fn run(
    provider: &LibkrunProvider,
    spec: VmSpec,
    binding: Option<&super::attempt::Binding>,
) -> Result<Arc<dyn VmInstance>, VmError> {
    if provider.inner.owner.get().is_none() {
        ownership::reject_unowned(&provider.inner.config)?;
    }
    let owner_guard = provider
        .inner
        .owner
        .get()
        .map(|owner| owner.validate(&provider.inner.config))
        .transpose()?;
    let prepared = prepare_spec(&provider.inner.config, &spec)?;
    if provider
        .inner
        .failed_cleanup
        .lock()
        .await
        .contains_key(&spec.id)
    {
        return Err(VmError::AlreadyExists(spec.id));
    }
    let (entry, retries) = PendingLaunch::new(
        spec.id.clone(),
        &provider.inner.config,
        owner_guard.as_ref(),
    )?;
    let (result, receiver) = oneshot::channel();
    {
        let mut ids = provider.inner.ids.lock().await;
        let mut pending = provider
            .inner
            .pending_launches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.contains_key(&entry.id) {
            return Err(VmError::AlreadyExists(entry.id.clone()));
        }
        if let Some(binding) = binding {
            binding.reserve(Arc::clone(&entry))?;
        }
        if let Err(error) =
            provider
                .inner
                .registry
                .admit(&provider.inner.lifetime, &mut ids, Arc::clone(&entry))
        {
            if let Some(binding) = binding {
                binding.not_admitted();
            }
            return Err(error);
        }
        if let Some(binding) = binding {
            binding.admitted();
        }
        pending.insert(entry.id.clone(), Arc::clone(&entry));
        // Reservation, discoverable custody and owning-task installation
        // have no await gap. Caller cancellation drops only its receiver.
        let task = tokio::spawn(launch::run_launch(
            Arc::clone(&provider.inner),
            Arc::clone(&entry),
            prepared,
            owner_guard,
            result,
            retries,
        ));
        if let Some(binding) = binding {
            binding.retain_launch_owner(task)?;
        } else {
            drop(task);
        }
        // None preserves the ordinary detached native owner; token Some keeps
        // its exact JoinHandle until positive final handoff/cleanup observation.
        drop(pending);
        drop(ids);
    }
    accept(provider, entry, receiver, binding).await
}

async fn accept(
    provider: &LibkrunProvider,
    entry: Arc<PendingLaunch>,
    receiver: oneshot::Receiver<Result<launch::LaunchHandoff, VmError>>,
    binding: Option<&super::attempt::Binding>,
) -> Result<Arc<dyn VmInstance>, VmError> {
    let launch::LaunchHandoff { instance, accepted } = receiver
        .await
        .map_err(|_| unavailable_error("worker launch", "launch result channel closed"))??;
    // Cancellation while acquiring this lock still drops the acknowledgement,
    // leaving the owner task responsible for the same actual instance.
    let ids = provider.inner.ids.lock().await;
    let mut pending = provider
        .inner
        .pending_launches
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !pending
        .get(&entry.id)
        .is_some_and(|current| Arc::ptr_eq(current, &entry))
    {
        return Err(VmError::InvalidState("launch handoff entry changed"));
    }
    if entry.drain_requested() {
        return Err(unavailable_error(
            "worker launch",
            "generation drain requested",
        ));
    }
    // Acknowledge before retiring only this entry; preserve the ID reservation.
    // No await separates acknowledgement, retirement and returning the instance.
    if let Some(binding) = binding {
        binding.instance(Arc::clone(&instance))?;
    }
    accepted
        .send(())
        .map_err(|()| unavailable_error("worker launch", "launch handoff channel closed"))?;
    pending.remove(&entry.id);
    entry.state.send_replace(launch::LaunchState::HandedOff);
    drop(entry);
    drop(pending);
    drop(ids);
    Ok(instance)
}
