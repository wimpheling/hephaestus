//! Controlled checked planning/retention; not native publication or VM proof.
use super::super::{Mode, admission};
use super::{model, preparation_model, request};
use admission::pending::{self, Endpoint};
use recipe_application::{
    DeploymentLifecycle, DeploymentSnapshot, PendingInstanceDrainClosure,
    PreparedInstanceClosureAdmission, ResourceProgress,
};
use std::collections::BTreeMap;

fn pending_model() -> (super::super::Request, PendingInstanceDrainClosure) {
    let mut current = request(11, Mode::Resume);
    current.remove.expected_version = 9;
    let (target, closure) = model();
    let snapshot = DeploymentSnapshot {
        intent: preparation_model().0,
        lifecycle: DeploymentLifecycle::Removing,
        version: 9,
        resources: BTreeMap::from([(
            closure.drain.resource.clone(),
            ResourceProgress {
                install: recipe_application::InstallProgress::Ready,
                removal: recipe_application::RemovalProgress::Draining,
                version: closure.drain.resource_version,
                active_attempt: Some(closure.drain.provenance.attempt_id),
                diagnostic: None,
            },
        )]),
    };
    let pending =
        PendingInstanceDrainClosure::new(snapshot, current.remove.command, target, closure.drain)
            .unwrap();
    (current, pending)
}

#[test]
fn pending_selection_never_masks_closed_history_or_ambiguity() {
    let (current, unpublished) = pending_model();
    let (_, closure) = model();
    assert!(matches!(
        pending::select(&[], std::slice::from_ref(&unpublished), &current.resource).unwrap(),
        Endpoint::Pending(_)
    ));
    assert!(matches!(
        pending::select(std::slice::from_ref(&closure), &[], &current.resource).unwrap(),
        Endpoint::Existing(_)
    ));
    assert!(
        pending::select(
            std::slice::from_ref(&closure),
            std::slice::from_ref(&unpublished),
            &current.resource
        )
        .is_err()
    );
    assert!(pending::select(&[closure.clone(), closure], &[], &current.resource).is_err());
    assert!(pending::select(&[], &[unpublished.clone(), unpublished], &current.resource).is_err());
}

#[test]
fn full_pending_plan_requires_exact_current_admission_target_and_active_claim() {
    let (current, unpublished) = pending_model();
    let snapshot = unpublished.snapshot();
    let target = unpublished.target();
    let active = unpublished.drain();
    let planned = pending::plan(&current, snapshot, target, Some(active), &unpublished).unwrap();
    assert_eq!(planned.pending(), &unpublished);
    assert_eq!(planned.expected_deployment_version(), 9);
    assert_eq!(
        planned.expected_instance_version(),
        current.expected_instance_version
    );
    assert!(pending::plan(&current, snapshot, target, None, &unpublished).is_err());
    let mut wrong = active.clone();
    wrong.generation += 1;
    assert!(pending::plan(&current, snapshot, target, Some(&wrong), &unpublished).is_err());
    let mut changed = snapshot.clone();
    changed.version += 1;
    assert!(pending::plan(&current, &changed, target, Some(active), &unpublished).is_err());
    let foreign = recipe_application::PreparedInstanceSafetyTarget::new(
        target.original().clone(),
        target.prepared().clone(),
        "foreign-owner".into(),
        target.host_id().to_owned(),
    )
    .unwrap();
    assert!(pending::plan(&current, snapshot, &foreign, Some(active), &unpublished).is_err());
    let mut other = request(12, Mode::Resume);
    other.remove.expected_version = 9;
    assert!(pending::plan(&other, snapshot, target, Some(active), &unpublished).is_err());
}

#[test]
fn exact_retry_reuses_retained_request_and_changed_actual_request_keeps_uncertain_original() {
    let (current, unpublished) = pending_model();
    let planned = pending::plan(
        &current,
        unpublished.snapshot(),
        unpublished.target(),
        Some(unpublished.drain()),
        &unpublished,
    )
    .unwrap();
    let mut original = admission::Original::new(current.clone());
    pending::retain(&mut original, &current.identity, planned.clone()).unwrap();
    assert_eq!(original.resumed_publications.len(), 1); // Retained before any commit future exists.
    pending::retain(&mut original, &current.identity, planned.clone()).unwrap();
    assert_eq!(original.resumed_publications.len(), 1);
    let conflicting = recipe_application::ResumedPreparedInstanceClosureRequest::new(
        planned.current_command(),
        planned.intent().clone(),
        planned.target().clone(),
        planned.drain().clone(),
        planned.pending().clone(),
        2,
    )
    .unwrap();
    assert!(pending::retain(&mut original, &current.identity, conflicting).is_err());
    assert_eq!(original.resumed_publications.len(), 1);
    let held = std::sync::Arc::new(super::super::PreparedInstanceDrainError::OutcomeUnknown);
    original.first_error = Some(held.clone());
    original.last_error = Some(held.clone());
    let mut changed = current.identity.clone();
    changed.request_id = identity_domain::RequestId::from_uuid(uuid::Uuid::from_u128(99));
    pending::retain(&mut original, &changed, planned.clone()).unwrap();
    assert_eq!(original.resumed_publications.len(), 2);
    assert_eq!(original.resumed_publications[0].request, planned);
    assert_ne!(
        original.resumed_publications[0].current.request_id,
        original.resumed_publications[1].current.request_id
    );
    assert!(std::sync::Arc::ptr_eq(
        original.first_error.as_ref().unwrap(),
        &held
    ));
    drop(held);
    let wrong_actor = request(12, Mode::Resume);
    assert!(pending::retain(&mut original, &wrong_actor.identity, planned).is_err());
    assert_eq!(original.resumed_publications.len(), 2);
}

#[test]
fn positive_winner_keeps_stored_cas_but_unpublished_cas_mismatch_denies() {
    let (mut current, unpublished) = pending_model();
    current.expected_instance_version = 2; // Stale explicit caller instance CAS.
    let planned = pending::plan(
        &current,
        unpublished.snapshot(),
        unpublished.target(),
        Some(unpublished.drain()),
        &unpublished,
    )
    .unwrap();
    let (_, closure) = model(); // Genuine provenance must come from native in production.
    assert_ne!(
        closure.expected_version,
        planned.expected_instance_version()
    );
    let mut observed = PreparedInstanceClosureAdmission {
        target: unpublished.target().clone(),
        removal_id: planned.removal_id(),
        replayed: true,
    };
    pending::compare_winner(&observed, &planned, &closure).unwrap();
    assert_eq!(closure.expected_version, 1); // Never overwrite it from caller CAS.
    observed.replayed = false;
    assert!(pending::compare_winner(&observed, &planned, &closure).is_err());
    current.remove.expected_version += 1;
    assert!(
        pending::plan(
            &current,
            unpublished.snapshot(),
            unpublished.target(),
            Some(unpublished.drain()),
            &unpublished
        )
        .is_err()
    );
    // Already-closed selection needs no reconstructed Pending/old admission CAS.
    assert!(matches!(
        pending::select(std::slice::from_ref(&closure), &[], &current.resource).unwrap(),
        Endpoint::Existing(_)
    ));
}

#[test]
fn lost_reply_fresh_tag_keeps_publisher_pins_and_requires_current_protected_match() {
    let (publisher, unpublished) = pending_model();
    let progress = recipe_application::InstanceClosurePublicationProgress::new(
        unpublished.drain().clone(),
        unpublished.snapshot().resources[&publisher.resource].clone(),
        unpublished.drain().generation,
        Some(recipe_application::ResourceAction::Create),
    )
    .unwrap();
    let publication_request = identity_domain::RequestId::from_uuid(uuid::Uuid::from_u128(99));
    let publication = recipe_application::QualifiedFreshInstanceClosurePublication::new(
        unpublished.drain().clone(),
        publisher.remove.command,
        publisher.remove.command.id(),
        publisher.identity.request_id,
        publication_request,
        unpublished.snapshot().version,
        progress,
    )
    .unwrap();
    let (_, mut closure) = model();
    closure.actor = publisher.identity.user_id;
    closure.request = publication_request;
    closure.publication_origin =
        recipe_application::InstanceClosurePublicationOrigin::QualifiedFreshRemove(Box::new(
            publication,
        ));
    let mut manager_c = request(12, Mode::Resume);
    manager_c.expected_instance_version = 2; // Stored publisher B used CAS 1.
    manager_c.remove.expected_version = 42; // Current admission, never B's saved 9.
    admission::compare_request(&manager_c, &admission::Original::new(manager_c.clone())).unwrap();
    let checked = recipe_application::ExistingInstanceClosureRequest::new(
        manager_c.remove.command,
        unpublished.snapshot().intent.clone(),
        unpublished.target().clone(),
        closure.clone(),
    )
    .unwrap();
    assert_eq!(checked.expected(), &closure);
    assert_ne!(closure.actor, manager_c.identity.user_id);
    assert_eq!(closure.expected_version, 1);
    assert!(matches!(
        pending::compare_loaded(
            &closure,
            Err(recipe_application::DeploymentError::AuthorizationDenied)
        ),
        Err(recipe_application::DeploymentError::AuthorizationDenied)
    ));
    for change in 0..3 {
        let mut wrong = closure.clone();
        match change {
            0 => wrong.expected_version += 1,
            1 => wrong.drain.generation += 1,
            _ => wrong.request = manager_c.identity.request_id,
        }
        assert!(pending::compare_loaded(&closure, Ok(wrong)).is_err());
    }
    // Controlled loader outcomes only: real production authenticates C first.
    pending::compare_loaded(&closure, Ok(closure.clone())).unwrap();
    assert_eq!(
        admission::existing_disposition(Some(unpublished.drain()), &closure).unwrap(),
        admission::ExistingDisposition::Active
    );
    let recipe_application::InstanceClosurePublicationOrigin::QualifiedFreshRemove(saved) =
        &closure.publication_origin
    else {
        panic!("explicit fresh publisher tag was lost");
    };
    assert_eq!(saved.command(), publisher.remove.command);
    assert_eq!(saved.admission_version(), 9);
    assert_eq!(saved.publication_request(), publication_request);
}

#[test]
fn advanced_recovery_pending_requires_its_own_full_current_admission() {
    let (mut current, unpublished) = pending_model();
    let mut snapshot = unpublished.snapshot().clone();
    snapshot.lifecycle = DeploymentLifecycle::RecoveryRequired;
    snapshot.version += 1;
    let progress = snapshot.resources.get_mut(&current.resource).unwrap();
    progress.removal = recipe_application::RemovalProgress::RecoveryRequired;
    progress.version += 1;
    progress.diagnostic = Some(recipe_application::DiagnosticCode::ProviderOutcomeUnknown);
    current.remove.expected_version = snapshot.version;
    let recovery = PendingInstanceDrainClosure::with_recovery_progress(
        snapshot.clone(),
        current.remove.command,
        unpublished.target().clone(),
        unpublished.drain().clone(),
    )
    .unwrap();
    assert_eq!(
        pending::plan(
            &current,
            &snapshot,
            recovery.target(),
            Some(recovery.drain()),
            &recovery,
        )
        .unwrap()
        .drain(),
        unpublished.drain()
    );
    current.remove.expected_version -= 1;
    assert!(
        pending::plan(
            &current,
            &snapshot,
            recovery.target(),
            Some(recovery.drain()),
            &recovery
        )
        .is_err()
    );
    assert!(
        pending::plan(
            &current,
            unpublished.snapshot(),
            recovery.target(),
            Some(recovery.drain()),
            &recovery
        )
        .is_err()
    );
}
