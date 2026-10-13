//! Registered owning tasks; no caller receives an abort or physical-custody handle.
use super::{
    ReadyRetentionCoordinator, ReadyRetentionError, ReadyRetentionPending, retained, validation,
    work,
};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{ResourceProgress, RetainOwnedReady};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{mpsc, oneshot};
use volume_trait::OwnedProvisioningClaim;

pub(super) type Registry = Mutex<BTreeMap<uuid::Uuid, Control>>;
type Result = std::result::Result<ResourceProgress, Arc<ReadyRetentionError>>;
pub(super) struct Control {
    original: retained::Request,
    status: ReadyRetentionPending,
    requests: mpsc::Sender<Delivery>,
}
struct Delivery {
    request: retained::Request,
    result: oneshot::Sender<Result>,
}

pub(super) fn inspect(registry: &Registry) -> Vec<ReadyRetentionPending> {
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .map(|control| control.status.clone())
        .collect()
}

pub(super) async fn submit(
    service: &ReadyRetentionCoordinator,
    identity: &AuthenticatedIdentity,
    request: &RetainOwnedReady,
    claim: &OwnedProvisioningClaim,
    retry_only: bool,
    handover: Option<super::handover::Comparison>,
) -> std::result::Result<ResourceProgress, ReadyRetentionError> {
    request
        .command
        .validate(identity, recipe_application::DeploymentOperation::Remove)?;
    let request = retained::Request {
        identity: identity.clone(),
        retention: request.clone(),
        claim: claim.clone(),
        scope: validation::scope(service.provider.as_ref())?,
        handover,
    };
    let key = request
        .claim
        .purpose
        .receipt()
        .intent
        .registration()
        .id()
        .as_uuid();
    let registry = service
        .pending
        .get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())));
    let (reply, completed) = oneshot::channel();
    let (requests, receiver) = register(registry, key, &request, retry_only)?;
    let delivery = Delivery {
        request,
        result: reply,
    };
    if let Some(receiver) = receiver {
        let context = Arc::new(work::Context {
            actor: service.actor.clone(),
            worker: service.worker.clone(),
            local: service.local.clone(),
            provider: service.provider.clone(),
        });
        // Actual tuple, exact original and retry intake registered BEFORE FIRST
        // await. The detached task owns probe/guard work, not the reply waiter.
        detach(own(
            context,
            Arc::downgrade(registry),
            key,
            receiver,
            delivery,
        ));
    } else if requests.send(delivery).await.is_err() {
        return Err(ReadyRetentionError::CompletionUnknown);
    }
    drop(requests);
    completed
        .await
        .map_err(|_| ReadyRetentionError::CompletionUnknown)?
        .map_err(ReadyRetentionError::Held)
}

fn register(
    registry: &Registry,
    key: uuid::Uuid,
    original: &retained::Request,
    retry_only: bool,
) -> std::result::Result<
    (mpsc::Sender<Delivery>, Option<mpsc::Receiver<Delivery>>),
    ReadyRetentionError,
> {
    let mut registry = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match registry.entry(key) {
        Entry::Occupied(entry) => {
            if !entry.get().original.exact(original)
                && !original.handover_from(&entry.get().original)
            {
                return Err(ReadyRetentionError::Correlation);
            }
            Ok((entry.get().requests.clone(), None))
        }
        Entry::Vacant(entry) => {
            if retry_only || original.handover.is_some() {
                return Err(ReadyRetentionError::Correlation);
            }
            let (requests, receiver) = mpsc::channel(1);
            entry.insert(Control {
                original: original.clone(),
                requests: requests.clone(),
                status: ReadyRetentionPending {
                    volume_id: key,
                    operation_id: original.retention.operation_id,
                    held: false,
                    physical_custody: false,
                },
            });
            Ok((requests, Some(receiver)))
        }
    }
}

fn detach(work: impl std::future::Future<Output = ()> + Send + 'static) {
    drop(tokio::spawn(work));
}

async fn own(
    context: Arc<work::Context>,
    registry: Weak<Registry>,
    key: uuid::Uuid,
    mut requests: mpsc::Receiver<Delivery>,
    initial: Delivery,
) {
    let mut original = retained::Original::new(initial.request.clone());
    let mut delivery = initial;
    loop {
        let result = Box::pin(work::retain(
            &context,
            &delivery.request,
            &mut original,
            &registry,
            key,
        ))
        .await
        .map_err(Arc::new);
        if let Err(error) = &result {
            original.errors.push(error.clone());
        }
        publish(&registry, key, &original, &result);
        let resolved = result.is_ok();
        let _ = delivery.result.send(result);
        if resolved {
            remove(&registry, key);
            return;
        }
        let Some(next) = requests.recv().await else {
            // Explicit owner/runtime loss is not successful release or physical
            // absence. Normal integration must retain this service while Held.
            tracing::error!(volume=%key, error=?original.errors.last(),
                physical_custody=original.custody.is_some(),
                "Ready retention coordinator dropped with unresolved original custody");
            return;
        };
        delivery = next;
    }
}

pub(super) fn custody_acquired(registry: &Weak<Registry>, key: uuid::Uuid) {
    if let Some(registry) = registry.upgrade() {
        let mut registry = registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(control) = registry.get_mut(&key) {
            control.status.physical_custody = true;
        }
    }
}

fn publish(
    registry: &Weak<Registry>,
    key: uuid::Uuid,
    original: &retained::Original,
    result: &Result,
) {
    if let Some(registry) = registry.upgrade() {
        let mut registry = registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(control) = registry.get_mut(&key) {
            control.original = original.active.clone();
            control.status.operation_id = original.active.retention.operation_id;
            control.status.held = result.is_err();
            control.status.physical_custody = original.custody.is_some();
        }
    }
}

fn remove(registry: &Weak<Registry>, key: uuid::Uuid) {
    if let Some(registry) = registry.upgrade() {
        registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&key);
    }
}

#[cfg(test)]
#[path = "owner/tests.rs"]
mod tests;
