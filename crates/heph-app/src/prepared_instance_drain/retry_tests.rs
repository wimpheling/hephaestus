//! Controlled request/operation retry checks; no PG or physical receipt proof.

use super::{model, request};
use crate::prepared_instance_drain::{
    Mode, PreparedInstanceDrainError, Request, admission, execution,
};
use identity_domain::{RequestId, UserId};
use recipe_application::{DeploymentOperation, EffectClaim, ResourceProgress};
use uuid::Uuid;

fn active_progress(claim: &EffectClaim) -> ResourceProgress {
    ResourceProgress {
        install: recipe_application::InstallProgress::Ready,
        removal: recipe_application::RemovalProgress::Draining,
        version: claim.resource_version,
        active_attempt: Some(claim.provenance.attempt_id),
        diagnostic: None,
    }
}

fn fresh_request(original: &Request, request_id: u128) -> Request {
    let mut fresh = original.clone();
    fresh.identity.request_id = RequestId::from_uuid(Uuid::from_u128(request_id));
    fresh
        .remove
        .command
        .validate(&fresh.identity, DeploymentOperation::Remove)
        .unwrap();
    fresh
}

#[test]
fn normal_completion_requires_exact_original_request_and_unchanged_progress() {
    let (_, closure) = model();
    let request = request(9, Mode::Original);
    let mut original = admission::Original::new(request.clone());
    original.claim = Some(closure.drain.clone());
    original.closure = Some(closure.clone());
    let progress = active_progress(&closure.drain);
    assert_eq!(
        execution::completion(&request, &closure.drain, &progress, &mut original).unwrap(),
        execution::Completion::Original,
    );
    let fresh = fresh_request(&request, 100);
    assert_eq!(fresh.remove, request.remove);
    let execution::Completion::Reconcile(provenance) =
        execution::completion(&fresh, &closure.drain, &progress, &mut original).unwrap()
    else {
        panic!("fresh transport request cannot become original completion")
    };
    assert_eq!(provenance.actor_id, fresh.identity.user_id);
    assert_eq!(provenance.request_id, fresh.identity.request_id);
    assert_ne!(provenance.attempt_id, closure.drain.provenance.attempt_id);
    let mut advanced = progress;
    advanced.version += 1;
    assert!(matches!(
        execution::completion(&request, &closure.drain, &advanced, &mut original).unwrap(),
        execution::Completion::Reconcile(_),
    ));
    assert_eq!(original.claim.as_ref(), Some(&closure.drain));
    assert_eq!(original.closure.as_ref(), Some(&closure));
}

#[test]
fn lost_worker_reply_reuses_exact_retained_operation_and_new_request_preserves_it() {
    // Controlled scheduling only: no forged PostgreSQL receipt or physical proof.
    let (_, closure) = model();
    let request = request(9, Mode::Original);
    let fresh = fresh_request(&request, 101);
    let mut original = admission::Original::new(request.clone());
    original.claim = Some(closure.drain.clone());
    original.closure = Some(closure.clone());
    let progress = active_progress(&closure.drain);
    let selected = execution::completion(&fresh, &closure.drain, &progress, &mut original).unwrap();
    assert_eq!(original.completions[0].completion, selected);
    let (reply, waiter) = tokio::sync::oneshot::channel();
    drop(waiter);
    assert_eq!(reply.send(selected), Err(selected)); // Original waiter lost the response.
    assert_eq!(
        execution::completion(&fresh, &closure.drain, &progress, &mut original).unwrap(),
        selected,
    );
    assert_eq!(original.completions.len(), 1);
    let held = std::sync::Arc::new(PreparedInstanceDrainError::OutcomeUnknown);
    original.last_error = Some(held.clone());
    execution::retain_completion_error(&mut original);
    execution::retain_completion_error(&mut original);
    let newer = fresh_request(&request, 102);
    let next = execution::completion(&newer, &closure.drain, &progress, &mut original).unwrap();
    let (execution::Completion::Reconcile(first), execution::Completion::Reconcile(second)) =
        (selected, next)
    else {
        panic!("both fresh requests require reconciliation")
    };
    assert_ne!(first.attempt_id, second.attempt_id);
    assert_eq!(second.request_id, newer.identity.request_id);
    assert_eq!(original.completions.len(), 2);
    assert_eq!(original.completions[0].completion, selected);
    assert_eq!(original.completion_errors.len(), 1);
    assert!(std::sync::Arc::ptr_eq(
        &original.completion_errors[0],
        &held
    ));
    assert_eq!(
        execution::completion(&fresh, &closure.drain, &progress, &mut original).unwrap(),
        selected,
    );
    assert_eq!(original.claim.as_ref(), Some(&closure.drain));
    assert_eq!(original.closure.as_ref(), Some(&closure));
}

#[test]
fn retained_operation_never_bypasses_active_claim_or_current_command_comparison() {
    let (_, closure) = model();
    let request = fresh_request(&request(9, Mode::Original), 103);
    let mut original = admission::Original::new(request.clone());
    original.claim = Some(closure.drain.clone());
    let progress = active_progress(&closure.drain);
    execution::completion(&request, &closure.drain, &progress, &mut original).unwrap();
    let mut inactive = progress.clone();
    inactive.active_attempt = None;
    assert!(execution::completion(&request, &closure.drain, &inactive, &mut original).is_err());
    let mut changed = closure.drain.clone();
    changed.generation += 1;
    assert!(execution::completion(&request, &changed, &progress, &mut original).is_err());
    let mut foreign = request.clone();
    foreign.identity.user_id = UserId::from_uuid(Uuid::from_u128(999));
    assert!(execution::completion(&foreign, &closure.drain, &progress, &mut original).is_err());
    assert_eq!(original.completions.len(), 1);
}
