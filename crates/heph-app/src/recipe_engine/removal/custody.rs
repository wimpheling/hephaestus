//! Deployment-serialized explicit ownership; no caller receives an abort handle.
use super::{
    PreparedRecipeRemovalCoordinator, RecipeRemovalError, RecipeRemovalPending, Request,
    completion, context::Context, retained::Original,
};
use recipe_application::{CommandReceipt, DeploymentId};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{mpsc, oneshot};
type Result = std::result::Result<CommandReceipt, Arc<RecipeRemovalError>>;
type Registry = Mutex<BTreeMap<DeploymentId, Control>>;
pub(super) struct Control {
    status: RecipeRemovalPending,
    requests: mpsc::Sender<Delivery>,
}
struct Delivery {
    request: Request,
    result: oneshot::Sender<Result>,
}

pub(super) fn inspect(registry: &Registry) -> Vec<RecipeRemovalPending> {
    registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .map(|control| control.status.clone())
        .collect()
}
pub(super) async fn submit(service: &PreparedRecipeRemovalCoordinator, request: Request) -> Result {
    request
        .remove
        .command
        .validate(
            &request.identity,
            recipe_application::DeploymentOperation::Remove,
        )
        .map_err(|error| Arc::new(error.into()))?;
    let key = request.remove.deployment_id;
    let (reply, completed) = oneshot::channel();
    let (requests, receiver) = register(&service.pending, key, &request);
    let delivery = Delivery {
        request,
        result: reply,
    };
    if let Some(receiver) = receiver {
        // Retain ORIGINAL tuple, input and registration BEFORE the FIRST await.
        drop(tokio::spawn(own(
            service.context.clone(),
            Arc::downgrade(&service.pending),
            key,
            receiver,
            delivery,
        )));
    } else if requests.send(delivery).await.is_err() {
        return Err(Arc::new(RecipeRemovalError::OutcomeUnknown));
    }
    drop(requests);
    completed
        .await
        .unwrap_or_else(|_| Err(Arc::new(RecipeRemovalError::OutcomeUnknown)))
}
fn register(
    registry: &Registry,
    key: DeploymentId,
    request: &Request,
) -> (mpsc::Sender<Delivery>, Option<mpsc::Receiver<Delivery>>) {
    let mut registry = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match registry.entry(key) {
        Entry::Occupied(entry) => (entry.get().requests.clone(), None),
        Entry::Vacant(entry) => {
            let (requests, receiver) = mpsc::channel(1);
            entry.insert(Control {
                requests: requests.clone(),
                status: RecipeRemovalPending {
                    deployment_id: key,
                    current_command: request.remove.command,
                    held: false,
                },
            });
            (requests, Some(receiver))
        }
    }
}
async fn own(
    context: Arc<Context>,
    registry: Weak<Registry>,
    key: DeploymentId,
    mut requests: mpsc::Receiver<Delivery>,
    initial: Delivery,
) {
    let mut original = Original::new(initial.request.clone());
    let mut delivery = initial;
    loop {
        let result = Box::pin(completion::run(&context, &delivery.request, &mut original))
            .await
            .map_err(Arc::new);
        if let Err(error) = &result {
            original.errors.push(error.clone());
        }
        if let Some(registry) = registry.upgrade() {
            if let Some(control) = registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_mut(&key)
            {
                control.status.current_command = delivery.request.remove.command;
                control.status.held = result.is_err();
            }
        }
        let resolved = result.is_ok();
        let _ = delivery.result.send(result);
        if resolved {
            if let Some(registry) = registry.upgrade() {
                registry
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&key);
            }
            return;
        }
        let Some(next) = requests.recv().await else {
            tracing::error!(deployment=%key, error=?original.errors.last(), "recipe removal owner dropped with held originals; no cleanup/absence guarantee");
            return;
        };
        delivery = next; // Never automatic retry or reconstructed saved principal.
    }
}
