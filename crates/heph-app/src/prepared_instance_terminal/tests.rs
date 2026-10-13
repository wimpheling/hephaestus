//! Checked model/control tests only; no native/PG/physical provenance claim.
#[path = "test_fixture.rs"]
pub(super) mod fixture;
use super::{
    Mode, PreparedInstanceTerminalError, admission, claims, completion, execution, retained,
};
use fixture::{delete, request};
use identity_domain::RequestId;
use recipe_application::{
    PreparedInstanceDeleteContinuation, PreparedInstanceDeleteHistory,
    PreparedInstanceDeleteObservation, PreparedInstanceDeletePublicationRecord,
    PreparedInstanceTerminalProgress, RemovalProgress, ResourceAction, ResourceProgress,
};
use uuid::Uuid;

fn progress(claim: &recipe_application::EffectClaim) -> ResourceProgress {
    ResourceProgress {
        install: recipe_application::InstallProgress::Ready,
        removal: RemovalProgress::Detached,
        version: claim.resource_version,
        active_attempt: Some(claim.provenance.attempt_id),
        diagnostic: None,
    }
}
#[test]
fn terminal_phase_never_turns_active_drain_or_missing_claim_into_delete() {
    let (intent, target, closure) = fixture::model();
    let plan = &intent.resources()[&target.original().resource];
    assert_eq!(plan.removal(), recipe_domain::RemovalPolicy::Delete);
    plan.validate_action(ResourceAction::Delete).unwrap();
    let mut p = progress(&closure.drain);
    assert!(matches!(
        admission::next(&p, Some(&closure.drain)),
        Err(PreparedInstanceTerminalError::DrainPending)
    ));
    assert!(admission::next(&p, None).is_err());
    p.active_attempt = None;
    p.removal = RemovalProgress::Pending;
    assert!(admission::next(&p, None).is_err());
    p.removal = RemovalProgress::Draining;
    assert_eq!(admission::next(&p, None).unwrap(), ResourceAction::Detach);
    p.removal = RemovalProgress::Detached;
    assert_eq!(admission::next(&p, None).unwrap(), ResourceAction::Delete);
    p.removal = RemovalProgress::Deleted;
    assert!(admission::next(&p, None).is_err()); // Requires separate positive history.
}
#[test]
fn original_completion_and_genuine_request_reconciliation_are_distinct() {
    let checked = delete();
    let claim = checked.effect();
    let req = request(9, Mode::Original);
    let mut owned = retained::Original::new(req.clone());
    owned.claim(claim).unwrap();
    let p = progress(claim);
    let i = completion::choose(&req, claim, &p, &mut owned).unwrap();
    assert_eq!(
        owned.completions[i].completion,
        completion::Completion::Original
    );
    let mut fresh = req.clone();
    fresh.identity.request_id = RequestId::from_uuid(Uuid::from_u128(99));
    let next = completion::choose(&fresh, claim, &p, &mut owned).unwrap();
    let completion::Completion::Reconcile(provenance) = owned.completions[next].completion else {
        panic!("changed authenticated request requires genuine reconciliation")
    };
    assert_eq!(provenance.request_id, fresh.identity.request_id);
    assert_ne!(provenance.attempt_id, claim.provenance.attempt_id);
    assert_eq!(
        completion::choose(&fresh, claim, &p, &mut owned).unwrap(),
        next
    );
    let mut advanced = p;
    advanced.version += 1;
    let advanced_operation = completion::choose(&req, claim, &advanced, &mut owned).unwrap();
    assert!(matches!(
        owned.completions[advanced_operation].completion,
        completion::Completion::Reconcile(_)
    ));
}
#[test]
fn lost_reply_retains_cas_proposal_operation_and_original_error() {
    let req = request(9, Mode::Original);
    let checked = delete();
    let mut owned = retained::Original::new(req.clone());
    owned.delete(&checked).unwrap();
    owned.claim(checked.effect()).unwrap();
    let proposal = claims::proposal(&req, &mut owned, 10, 6, ResourceAction::Delete).unwrap();
    let same = claims::proposal(&req, &mut owned, 10, 6, ResourceAction::Delete).unwrap();
    assert_eq!(proposal.provenance, same.provenance);
    assert_eq!(owned.proposals.len(), 1);
    let p = progress(checked.effect());
    let i = completion::choose(&req, checked.effect(), &p, &mut owned).unwrap();
    let (reply, waiter) = tokio::sync::oneshot::channel::<usize>();
    drop(waiter);
    assert_eq!(reply.send(i), Err(i));
    assert_eq!(
        completion::choose(&req, checked.effect(), &p, &mut owned).unwrap(),
        i
    );
    let error = std::sync::Arc::new(PreparedInstanceTerminalError::OutcomeUnknown);
    owned.errors.push(error.clone());
    let mut fresh = req;
    fresh.identity.request_id = RequestId::from_uuid(Uuid::from_u128(101));
    let newer = claims::proposal(&fresh, &mut owned, 10, 6, ResourceAction::Delete).unwrap();
    assert_ne!(proposal.provenance.attempt_id, newer.provenance.attempt_id);
    assert_eq!(owned.proposals[0].provenance, proposal.provenance);
    assert_eq!(owned.delete[0].expected_instance_version(), 3);
    assert!(std::sync::Arc::ptr_eq(&owned.errors[0], &error));
}
fn committed(applied: bool) -> PreparedInstanceDeleteHistory {
    let req = delete();
    let published =
        PreparedInstanceDeleteObservation::new(req.clone(), Uuid::from_u128(50), 4).unwrap();
    let record = PreparedInstanceDeletePublicationRecord::new(
        published,
        req.current_command(),
        req.current_command().id(),
        RequestId::from_uuid(Uuid::from_u128(19)),
    )
    .unwrap();
    let continuation = PreparedInstanceDeleteContinuation::committed(req.clone(), record).unwrap();
    let mut p = progress(req.effect());
    let action = if applied {
        p.active_attempt = None;
        p.removal = RemovalProgress::Deleted;
        p.version += 1;
        ResourceAction::Delete
    } else {
        ResourceAction::Detach
    };
    let observed =
        PreparedInstanceTerminalProgress::new(p, req.effect().generation, Some(action)).unwrap();
    if applied {
        PreparedInstanceDeleteHistory::completed(
            recipe_application::PreparedInstanceDeleteCompletedHistory::new(
                continuation,
                observed,
                None,
            )
            .unwrap(),
        )
    } else {
        PreparedInstanceDeleteHistory::active(continuation, observed, None).unwrap()
    }
}
#[test]
fn inactive_applied_is_read_only_and_committed_active_is_observe_only() {
    assert_eq!(
        execution::plan(&committed(false)),
        execution::DeletePlan::Observe
    );
    assert_eq!(
        execution::plan(&committed(true)),
        execution::DeletePlan::Completed
    );
    let req = delete();
    let observed = PreparedInstanceTerminalProgress::new(
        progress(req.effect()),
        req.effect().generation,
        Some(ResourceAction::Detach),
    )
    .unwrap();
    let unwritten = PreparedInstanceDeleteHistory::active(
        PreparedInstanceDeleteContinuation::unwritten(req),
        observed,
        None,
    )
    .unwrap();
    assert_eq!(execution::plan(&unwritten), execution::DeletePlan::Publish);
}
#[test]
fn retained_original_cas_is_not_replaced_by_current_tombstone_version() {
    let original = delete();
    let mut owned = retained::Original::new(request(9, Mode::Original));
    owned.delete(&original).unwrap();
    let changed = recipe_application::PreparedInstanceDeleteRequest::new(
        original.intent().clone(),
        original.target().clone(),
        original.closure().clone(),
        original.effect().clone(),
        original.current_command(),
        original.configuration().clone(),
        4,
    )
    .unwrap();
    assert!(owned.delete(&changed).is_err());
    assert_eq!(owned.delete[0], original);
    let observation =
        PreparedInstanceDeleteObservation::new(original.clone(), Uuid::from_u128(50), 4).unwrap();
    owned.publication(&observation, false).unwrap();
    let wrong = PreparedInstanceDeleteObservation::new(original, Uuid::from_u128(51), 4).unwrap();
    assert!(owned.publication(&wrong, true).is_err());
}
#[test]
fn changed_scope_claim_or_current_actor_cannot_reuse_completion() {
    let checked = delete();
    let req = request(9, Mode::Original);
    let mut owned = retained::Original::new(req.clone());
    owned.claim(checked.effect()).unwrap();
    let p = progress(checked.effect());
    let mut changed = checked.effect().clone();
    changed.generation += 1;
    assert!(completion::choose(&req, &changed, &p, &mut owned).is_err());
    let mut wrong = req;
    wrong.identity.user_id = identity_domain::UserId::from_uuid(Uuid::from_u128(999));
    assert!(completion::choose(&wrong, checked.effect(), &p, &mut owned).is_err());
    assert!(owned.completions.is_empty());
}

#[test]
fn held_detach_observation_cannot_advance_to_delete() {
    let checked = delete();
    let caller = request(9, Mode::Original);
    let detach_claim = recipe_application::EffectClaim {
        action: ResourceAction::Detach,
        generation: 3,
        resource_version: 5,
        provenance: recipe_application::AttemptProvenance::new(
            &caller.identity,
            recipe_application::DeploymentAttemptId::from_uuid(Uuid::from_u128(35)).unwrap(),
        )
        .unwrap(),
        ..checked.effect().clone()
    };
    let detach = recipe_application::PreparedInstanceDetachRequest::new(
        checked.intent().clone(),
        checked.target().clone(),
        checked.closure().clone(),
        detach_claim.clone(),
        caller.remove.command,
        checked.configuration().clone(),
    )
    .unwrap();
    let held = recipe_application::PreparedInstanceSafetyObservation {
        target: checked.target().clone(),
        state: recipe_application::PreparedInstanceSafetyState::ClosedHeld,
        removal_id: Some(checked.closure().removal_id),
        inventory: recipe_application::InstanceSafetyInventory::default(),
    };
    assert!(recipe_application::PreparedInstanceDetachObservation::new(detach, held).is_err());
    let mut current = progress(&detach_claim);
    current.removal = RemovalProgress::Draining;
    assert_eq!(
        admission::next(&current, Some(&detach_claim)).unwrap(),
        ResourceAction::Detach
    );
    assert!(admission::next(&current, None).is_err());
}

#[test]
fn lost_claim_reply_requires_the_exact_proposed_attempt() {
    let req = request(9, Mode::Original);
    let checked = delete();
    let mut owned = retained::Original::new(req.clone());
    let proposal = claims::proposal(&req, &mut owned, 10, 6, ResourceAction::Delete).unwrap();
    let restored = recipe_application::EffectClaim {
        provenance: proposal.provenance,
        ..checked.effect().clone()
    };
    claims::restore_loaded(&mut owned, &restored).unwrap();
    assert_eq!(owned.claims[0], restored);
    let mut substituted = restored;
    substituted.provenance.attempt_id =
        recipe_application::DeploymentAttemptId::from_uuid(Uuid::from_u128(999)).unwrap();
    assert!(claims::restore_loaded(&mut owned, &substituted).is_err());
    assert_eq!(owned.proposals.len(), 1);
}

#[test]
fn positive_committed_publication_cannot_regress_to_unwritten() {
    let history = committed(false);
    let checked = history.original().original();
    let mut owned = retained::Original::new(request(9, Mode::Original));
    owned.history(&history).unwrap();
    let unwritten = PreparedInstanceDeleteHistory::active(
        PreparedInstanceDeleteContinuation::unwritten(checked.clone()),
        history.observed_progress().clone(),
        None,
    )
    .unwrap();
    assert!(owned.history(&unwritten).is_err());
    assert_eq!(owned.histories[0], history);
    assert_eq!(
        owned.publications[0].terminal_operation_id(),
        Uuid::from_u128(50)
    );
}
