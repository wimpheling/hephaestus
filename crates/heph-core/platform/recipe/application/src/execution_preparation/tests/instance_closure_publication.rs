//! Controlled substitution tests; no current rights or native history are certified.
mod fixture;
use self::fixture::{Exact, command};
use super::fixture as model;
use crate::{
    DeploymentLifecycle, DeploymentOperation, DiagnosticCode, ExistingInstanceClosureRequest,
    InstallProgress, InstanceClosurePublicationOrigin, InstanceClosurePublicationProgress,
    PendingInstanceDrainClosure, QualifiedFreshInstanceClosurePublication, RemovalProgress,
    ResourceAction,
};
use identity_domain::RequestId;
use release_domain::ContentHash;
use uuid::Uuid;

#[test]
fn original_drain_keeps_strict_actor_and_stable_identity_checks() {
    let exact = Exact::new();
    exact.closure.validate(&exact.drain).unwrap();
    for change in 0..5 {
        let mut closure = exact.closure.clone();
        match change {
            0 => {
                closure.actor = exact.publisher.actor_id();
            }
            1 => {
                closure.expected_version = 0;
            }
            2 => {
                closure.drain.action = ResourceAction::Detach;
            }
            3 => {
                closure.drain.provenance.actor_id = exact.publisher.actor_id();
            }
            _ => {
                closure.drain.input_hash = ContentHash::digest(b"substituted intent");
            }
        }
        assert!(closure.validate(&exact.drain).is_err());
    }
}

#[test]
fn recorded_publisher_b_is_independent_of_current_cleanup_user_c() {
    let exact = Exact::new();
    let closure = exact.fresh_closure();
    closure.validate(&exact.drain).unwrap();
    let InstanceClosurePublicationOrigin::QualifiedFreshRemove(publication) =
        &closure.publication_origin
    else {
        panic!("fresh tag lost")
    };
    assert_eq!(publication.drain(), &exact.drain);
    assert_eq!(publication.command(), exact.publisher);
    assert_eq!(publication.native_command_id(), exact.publisher.id());
    assert_eq!(
        publication.admission_request(),
        model::identity(11, 12).request_id
    );
    assert_eq!(
        publication.publication_request(),
        model::identity(11, 15).request_id
    );
    assert_eq!(publication.admission_version(), exact.snapshot.version);
    assert_eq!(publication.pending(), &exact.progress());
    assert_eq!(publication.pending().generation(), exact.drain.generation);
    assert_eq!(
        publication.pending().last_action(),
        Some(ResourceAction::Create)
    );
    let current = command(13, 14);
    let request = ExistingInstanceClosureRequest::new(
        current,
        exact.snapshot.intent,
        exact.target,
        closure.clone(),
    )
    .unwrap();
    assert_eq!(request.command(), current);
    assert_ne!(request.command().actor_id(), closure.actor);
    assert_eq!(request.expected(), &closure);
    assert_eq!(closure.drain, exact.drain);
}

#[test]
fn fresh_publication_rejects_substituted_original_command_and_admission_pins() {
    let exact = Exact::new();
    for change in 0..8 {
        let mut drain = exact.drain.clone();
        let mut publisher = exact.publisher;
        let mut native = publisher.id();
        let mut admission = model::identity(11, 12).request_id;
        let mut publication = model::identity(11, 15).request_id;
        let mut version = 9;
        match change {
            0 => {
                drain.generation += 1;
            }
            1 => {
                publisher = exact.drain.command;
                native = publisher.id();
            }
            2 => {
                publisher = crate::CommandIdentity::from_identity(
                    &model::identity(11, 12),
                    DeploymentOperation::Install,
                )
                .unwrap();
                native = publisher.id();
            }
            3 => {
                native = command(13, 14).id();
            }
            4 => {
                admission = RequestId::from_uuid(Uuid::nil());
            }
            5 => {
                publication = RequestId::from_uuid(Uuid::nil());
            }
            6 => {
                version = 0;
            }
            _ => {
                version = i64::MAX.cast_unsigned() + 1;
            }
        }
        assert!(
            QualifiedFreshInstanceClosurePublication::new(
                drain,
                publisher,
                native,
                admission,
                publication,
                version,
                exact.progress(),
            )
            .is_err()
        );
    }
}

#[test]
fn fresh_closure_requires_its_exact_tag_publisher_request_and_full_drain() {
    let exact = Exact::new();
    for change in 0..4 {
        let mut closure = exact.fresh_closure();
        match change {
            0 => {
                closure.publication_origin = InstanceClosurePublicationOrigin::OriginalDrain;
            }
            1 => {
                closure.actor = model::identity(13, 14).user_id;
            }
            2 => {
                closure.request = model::identity(11, 12).request_id;
            }
            _ => {
                closure.drain.generation += 1;
            }
        }
        assert!(closure.validate(&exact.drain).is_err());
    }
}

#[test]
fn publication_progress_preserves_native_generation_create_and_active_attempt() {
    let exact = Exact::new();
    for change in 0..8 {
        let mut progress = exact.snapshot.resources[&exact.drain.resource].clone();
        let mut generation = exact.drain.generation;
        let mut last = Some(ResourceAction::Create);
        match change {
            0 => {
                generation += 1;
            }
            1 => {
                last = Some(ResourceAction::Drain);
            }
            2 => {
                progress.active_attempt = None;
            }
            3 => {
                progress.install = InstallProgress::Pending;
            }
            4 => {
                progress.version += 1;
            }
            5 => {
                progress.diagnostic = Some(DiagnosticCode::DrainPending);
            }
            6 => {
                progress.removal = RemovalProgress::Detached;
            }
            _ => {
                last = None;
            }
        }
        assert!(
            InstanceClosurePublicationProgress::new(
                exact.drain.clone(),
                progress,
                generation,
                last,
            )
            .is_err()
        );
    }
    let mut advanced = exact.snapshot.resources[&exact.drain.resource].clone();
    advanced.version += 1;
    advanced.removal = RemovalProgress::RecoveryRequired;
    advanced.diagnostic = Some(DiagnosticCode::ProviderOutcomeUnknown);
    let observed = InstanceClosurePublicationProgress::new(
        exact.drain.clone(),
        advanced.clone(),
        exact.drain.generation,
        Some(ResourceAction::Create),
    )
    .unwrap();
    assert_eq!(observed.progress(), &advanced);
    assert_eq!(observed.drain(), &exact.drain);
}

#[test]
fn recovery_pending_retains_advanced_snapshot_without_relaxing_strict_new() {
    let exact = Exact::new();
    let mut snapshot = exact.snapshot.clone();
    snapshot.lifecycle = DeploymentLifecycle::RecoveryRequired;
    let progress = snapshot.resources.get_mut(&exact.drain.resource).unwrap();
    progress.version += 1;
    progress.removal = RemovalProgress::RecoveryRequired;
    progress.diagnostic = Some(DiagnosticCode::ProviderOutcomeUnknown);
    let pending = PendingInstanceDrainClosure::with_recovery_progress(
        snapshot.clone(),
        exact.publisher,
        exact.target.clone(),
        exact.drain.clone(),
    )
    .unwrap();
    assert_eq!(pending.snapshot(), &snapshot);
    assert_eq!(pending.drain(), &exact.drain);
    assert_eq!(pending.target(), &exact.target);
    assert_eq!(pending.current_command(), exact.publisher);
    assert!(
        PendingInstanceDrainClosure::new(
            snapshot.clone(),
            exact.publisher,
            exact.target.clone(),
            exact.drain.clone(),
        )
        .is_err()
    );
    for change in 0..7 {
        let mut altered = snapshot.clone();
        let progress = altered.resources.get_mut(&exact.drain.resource).unwrap();
        match change {
            0 => {
                altered.lifecycle = DeploymentLifecycle::Removing;
            }
            1 => {
                progress.removal = RemovalProgress::Draining;
            }
            2 => {
                progress.diagnostic = None;
            }
            3 => {
                progress.active_attempt = None;
            }
            4 => {
                progress.version = exact.drain.resource_version;
            }
            5 => {
                progress.install = InstallProgress::Pending;
            }
            _ => {
                altered.intent = model::intent(model::SOURCE, 2);
            }
        }
        assert!(
            PendingInstanceDrainClosure::with_recovery_progress(
                altered,
                exact.publisher,
                exact.target.clone(),
                exact.drain.clone(),
            )
            .is_err()
        );
    }
    assert!(
        PendingInstanceDrainClosure::with_recovery_progress(
            snapshot,
            exact.drain.command,
            exact.target,
            exact.drain,
        )
        .is_err()
    );
}
