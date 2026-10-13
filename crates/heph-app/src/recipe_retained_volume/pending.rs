//! An unresolved actor outcome keeps actual custody in its detached task.

use identity_domain::AuthenticatedIdentity;
use recipe_application::{EffectClaim, ResourceProgress, RetainedExternalVerificationPlan};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::MutexGuard,
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use volume_local::OwnedRetainedReuseCustody;

use super::{
    PendingRetainedVolumeVerification, PreparedRetainedVolumeVerifier,
    RetainedVolumeVerificationError, completion,
};

type ResultSender = oneshot::Sender<Result<ResourceProgress, RetainedVolumeVerificationError>>;
pub(super) struct Control {
    claim: EffectClaim,
    ambiguous: bool,
    requests: mpsc::Sender<Request>,
}
struct Request {
    identity: AuthenticatedIdentity,
    result: ResultSender,
}

fn registry(service: &PreparedRetainedVolumeVerifier) -> MutexGuard<'_, BTreeMap<Uuid, Control>> {
    // No registry mutation runs user code. Preserve owners even after unrelated unwinding.
    service
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn inspect(service: &PreparedRetainedVolumeVerifier) -> Vec<PendingRetainedVolumeVerification> {
    registry(service)
        .values()
        .map(|control| PendingRetainedVolumeVerification {
            claim: control.claim.clone(),
            ambiguous: control.ambiguous,
        })
        .collect()
}

pub async fn retry(
    service: &PreparedRetainedVolumeVerifier,
    identity: &AuthenticatedIdentity,
    claim: &EffectClaim,
) -> Option<Result<ResourceProgress, RetainedVolumeVerificationError>> {
    let requests = {
        let registry = registry(service);
        let control = registry.get(&claim.provenance.attempt_id.as_uuid())?;
        if control.claim != *claim {
            return Some(Err(RetainedVolumeVerificationError::Correlation));
        }
        let requests = control.requests.clone();
        drop(registry);
        requests
    };
    let (result, completed) = oneshot::channel();
    if requests
        .send(Request {
            identity: identity.clone(),
            result,
        })
        .await
        .is_err()
    {
        return Some(Err(RetainedVolumeVerificationError::CompletionUnknown));
    }
    Some(
        completed
            .await
            .unwrap_or(Err(RetainedVolumeVerificationError::CompletionUnknown)),
    )
}

pub fn start(
    service: PreparedRetainedVolumeVerifier,
    identity: AuthenticatedIdentity,
    plan: RetainedExternalVerificationPlan,
    custody: OwnedRetainedReuseCustody,
) -> Result<
    oneshot::Receiver<Result<ResourceProgress, RetainedVolumeVerificationError>>,
    RetainedVolumeVerificationError,
> {
    let (requests, receiver) = mpsc::channel(1);
    let (result, completed) = oneshot::channel();
    {
        let mut registry = registry(&service);
        let Entry::Vacant(entry) = registry.entry(plan.claim.provenance.attempt_id.as_uuid())
        else {
            return Err(RetainedVolumeVerificationError::Correlation);
        };
        entry.insert(Control {
            claim: plan.claim.clone(),
            ambiguous: false,
            requests,
        });
        drop(registry);
    }
    // Dropping the handle detaches; callers receive replies without an abort handle.
    drop(tokio::spawn(own(
        service, identity, plan, custody, receiver, result,
    )));
    Ok(completed)
}

async fn own(
    service: PreparedRetainedVolumeVerifier,
    identity: AuthenticatedIdentity,
    plan: RetainedExternalVerificationPlan,
    mut custody: OwnedRetainedReuseCustody,
    mut requests: mpsc::Receiver<Request>,
    initial_result: ResultSender,
) {
    let resolution = completion::commit(&service, &identity, &plan, &mut custody).await;
    let mut resolved = publish(&service, &plan, resolution, initial_result);
    // This task's owning service retains the request sender while unresolved.
    // Dropping a caller, reply receiver or service handle cannot close this loop.
    while !resolved {
        if let Some(request) = requests.recv().await {
            let resolution = completion::resolve_pending(&service, &request.identity, &plan).await;
            resolved = publish(&service, &plan, resolution, request.result);
        } else {
            // Losing a recovery channel does not prove the actor transaction rolled back.
            std::future::pending::<()>().await;
        }
    }
    drop(custody); // Only exact paired commit or positively qualified noncommit releases custody.
    drop(service); // Actual provider, Local/root and role service Arcs survive every recovery await.
}

fn publish(
    service: &PreparedRetainedVolumeVerifier,
    plan: &RetainedExternalVerificationPlan,
    resolution: completion::Resolution,
    result: ResultSender,
) -> bool {
    let mut registry = registry(service);
    let key = plan.claim.provenance.attempt_id.as_uuid();
    if resolution.resolved {
        registry.remove(&key);
    } else if let Some(control) = registry.get_mut(&key) {
        control.ambiguous = true;
    }
    let _ = result.send(resolution.result);
    drop(registry);
    resolution.resolved
}
