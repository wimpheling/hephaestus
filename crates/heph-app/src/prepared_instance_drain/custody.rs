//! Registered per-resource work; a caller receives no task abort handle.

use super::{
    Context, PreparedInstanceDrainCoordinator, PreparedInstanceDrainError,
    PreparedInstanceDrainPending, Request, admission, execution,
};
use capability_domain::CapabilitySlotKey;
use recipe_application::{DeploymentId, ResourceProgress};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{mpsc, oneshot};

type Result = std::result::Result<ResourceProgress, Arc<PreparedInstanceDrainError>>;
pub(super) type Key = (DeploymentId, CapabilitySlotKey);
type Registry = Mutex<BTreeMap<Key, Control>>;

pub(super) struct Control {
    status: PreparedInstanceDrainPending,
    requests: mpsc::Sender<Delivery>,
}
struct Delivery {
    request: Request,
    result: oneshot::Sender<Result>,
}

pub(super) fn inspect(registry: &Registry) -> Vec<PreparedInstanceDrainPending> {
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .map(|control| control.status.clone())
        .collect()
}

pub(super) async fn submit(service: &PreparedInstanceDrainCoordinator, request: Request) -> Result {
    request
        .remove
        .command
        .validate(
            &request.identity,
            recipe_application::DeploymentOperation::Remove,
        )
        .map_err(|error| Arc::new(error.into()))?;
    if request.expected_instance_version == 0
        || request.expected_instance_version > i64::MAX.cast_unsigned()
    {
        return Err(Arc::new(
            recipe_application::DeploymentError::StaleClaim.into(),
        ));
    }
    let (result, completed) = oneshot::channel();
    let key = (request.remove.deployment_id, request.resource.clone());
    let (requests, receiver) = register(&service.pending, &key, &request);
    let delivery = Delivery { request, result };
    if let Some(receiver) = receiver {
        // Registration and all actual original inputs precede the FIRST await.
        // No sender is retained by this task: explicit service ownership keeps
        // held retry intake reachable, without a self-sustaining sender cycle.
        detach(own(
            service.context.clone(),
            Arc::downgrade(&service.pending),
            key,
            receiver,
            delivery,
        ));
    } else if requests.send(delivery).await.is_err() {
        return Err(Arc::new(PreparedInstanceDrainError::OutcomeUnknown));
    }
    drop(requests);
    completed
        .await
        .unwrap_or_else(|_| Err(Arc::new(PreparedInstanceDrainError::OutcomeUnknown)))
}

fn register(
    registry: &Registry,
    key: &Key,
    request: &Request,
) -> (mpsc::Sender<Delivery>, Option<mpsc::Receiver<Delivery>>) {
    let mut registry = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match registry.entry(key.clone()) {
        Entry::Occupied(entry) => (entry.get().requests.clone(), None),
        Entry::Vacant(entry) => {
            let (requests, receiver) = mpsc::channel(1);
            entry.insert(Control {
                requests: requests.clone(),
                status: PreparedInstanceDrainPending {
                    deployment_id: key.0,
                    resource: key.1.clone(),
                    original_command: request.remove.command,
                    attempt: None,
                    held: false,
                },
            });
            (requests, Some(receiver))
        }
    }
}

fn remove(registry: &Weak<Registry>, key: &Key) {
    if let Some(registry) = registry.upgrade() {
        registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key);
    }
}

fn detach(work: impl std::future::Future<Output = ()> + Send + 'static) {
    drop(tokio::spawn(work));
}

async fn own(
    context: Arc<Context>,
    registry: Weak<Registry>,
    key: Key,
    mut requests: mpsc::Receiver<Delivery>,
    initial: Delivery,
) {
    let mut original = admission::Original::new(initial.request.clone());
    let mut delivery = initial;
    loop {
        let result = Box::pin(execution::drain(&context, &delivery.request, &mut original))
            .await
            .map_err(Arc::new);
        if original.first_error.is_none() {
            original.first_error = result.as_ref().err().cloned();
        }
        original.last_error = result.as_ref().err().cloned();
        publish(&registry, &key, &original, &result);
        let resolved = result.is_ok();
        if resolved {
            remove(&registry, &key);
            let _ = delivery.result.send(result);
            break;
        }
        let _ = delivery.result.send(result);
        let Some(next) = requests.recv().await else {
            // Last explicit service owner disappeared. There is no Drop cleanup
            // guarantee or false absence; process crash/recovery uses durable originals.
            tracing::error!(deployment=%key.0, resource=%key.1.as_str(),
                error=?original.last_error, "prepared instance Drain retry owner dropped with held originals");
            return;
        };
        delivery = next;
    }
}

fn publish(registry: &Weak<Registry>, key: &Key, original: &admission::Original, result: &Result) {
    if let Some(registry) = registry.upgrade() {
        let mut registry = registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(control) = registry.get_mut(key) {
            control.status.held = result.is_err();
            control.status.attempt = original
                .claim
                .as_ref()
                .map(|claim| claim.provenance.attempt_id);
        }
    }
}

#[cfg(test)]
#[path = "custody_tests.rs"]
mod tests;
