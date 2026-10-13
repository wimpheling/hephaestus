use super::{fixture, instance_safety};
use crate::{
    AdmissionDisposition, AttemptProvenance, CommandIdentity, CommandReceipt, DeploymentAdmission,
    DeploymentAttemptId, DeploymentError, DeploymentLifecycle, DeploymentOperation,
    DeploymentSnapshot, EffectClaim, InstallProgress, OriginalInstanceClosure,
    PendingInstanceDrainClosure, PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyTarget, RemovalProgress, RemoveContinuation, RemoveResumeAdmission,
    ResourceAction, ResourceProgress, ResumedPreparedInstanceClosureRequest,
};
use async_trait::async_trait;
use release_domain::ContentHash;
use std::{
    collections::BTreeMap,
    future::Future,
    task::{Context, Poll, Waker},
};
use uuid::Uuid;

struct PendingFixture {
    admission: DeploymentAdmission,
    target: PreparedInstanceSafetyTarget,
    drain: EffectClaim,
    pending: PendingInstanceDrainClosure,
}

fn pending_fixture() -> PendingFixture {
    let target = instance_safety::target();
    let original_actor = fixture::identity(9, 10);
    let current_actor = fixture::identity(11, 12);
    let drain = EffectClaim {
        command: CommandIdentity::from_identity(&original_actor, DeploymentOperation::Remove)
            .unwrap(),
        action: ResourceAction::Drain,
        generation: 2,
        resource_version: 3,
        provenance: AttemptProvenance::new(
            &original_actor,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(50)).unwrap(),
        )
        .unwrap(),
        ..target.original().clone()
    };
    let intent = fixture::intent(fixture::SOURCE, 1);
    let current =
        CommandIdentity::from_identity(&current_actor, DeploymentOperation::Remove).unwrap();
    let snapshot = DeploymentSnapshot {
        intent: intent.clone(),
        lifecycle: DeploymentLifecycle::Removing,
        version: 9,
        resources: BTreeMap::from([(
            drain.resource.clone(),
            ResourceProgress {
                install: InstallProgress::Ready,
                removal: RemovalProgress::Draining,
                version: drain.resource_version,
                active_attempt: Some(drain.provenance.attempt_id),
                diagnostic: None,
            },
        )]),
    };
    let pending =
        PendingInstanceDrainClosure::new(snapshot.clone(), current, target.clone(), drain.clone())
            .unwrap();
    let admission = DeploymentAdmission {
        disposition: AdmissionDisposition::Resume,
        snapshot,
        receipt: CommandReceipt {
            command: current,
            deployment_id: intent.id(),
            input_hash: ContentHash::digest(b"controlled fresh admission"),
            lifecycle: DeploymentLifecycle::Removing,
            deployment_version: 9,
            event: event_application::CommittedMutation {
                event_id: Uuid::from_u128(90),
                scope_kind: "project".into(),
                scope_id: intent.project_id().as_uuid(),
                cursor: 1,
                aggregate_version: 1,
            },
        },
    };
    PendingFixture {
        admission,
        target,
        drain,
        pending,
    }
}

fn continuation(exact: &PendingFixture) -> RemoveContinuation {
    RemoveContinuation {
        claim: exact.drain.clone(),
        closure: None,
    }
}

fn closure(exact: &PendingFixture) -> OriginalInstanceClosure {
    let request = PreparedInstanceClosureRequest::new(
        exact.admission.snapshot.intent.clone(),
        exact.target.clone(),
        exact.drain.clone(),
        1,
    )
    .unwrap();
    OriginalInstanceClosure {
        publication_origin: crate::InstanceClosurePublicationOrigin::OriginalDrain,
        drain: exact.drain.clone(),
        removal_id: request.removal_id(),
        command_key: request.command_key(),
        expected_version: 1,
        actor: exact.drain.provenance.actor_id,
        request: exact.drain.provenance.request_id,
        input_hash: ContentHash::digest(b"controlled original closure"),
    }
}

#[test]
fn explicit_pending_preserves_original_claim_and_old_constructors_stay_strict() {
    let exact = pending_fixture();
    let active = continuation(&exact);
    assert!(matches!(
        RemoveResumeAdmission::new(exact.admission.clone(), vec![active.clone()]),
        Err(DeploymentError::ReconciliationRequired)
    ));
    assert!(matches!(
        RemoveResumeAdmission::with_complete_closures(
            exact.admission.clone(),
            vec![active.clone()],
            vec![],
        ),
        Err(DeploymentError::ReconciliationRequired)
    ));
    let compared = RemoveResumeAdmission::with_pending_instance_drains(
        exact.admission.clone(),
        vec![active.clone()],
        vec![],
        vec![exact.pending.clone()],
    )
    .unwrap();
    assert_eq!(compared.continuations(), &[active]);
    assert_eq!(
        compared.pending_instance_drains(),
        std::slice::from_ref(&exact.pending),
    );
    assert!(compared.closures().is_empty());
    let original = closure(&exact);
    let closed = RemoveContinuation {
        claim: exact.drain,
        closure: Some(original.clone()),
    };
    let strict = RemoveResumeAdmission::with_complete_closures(
        exact.admission,
        vec![closed],
        vec![original.clone()],
    )
    .unwrap();
    assert_eq!(strict.closures(), &[original]);
    assert!(strict.pending_instance_drains().is_empty());
}

#[test]
fn pending_rejects_replaced_active_attempt_version_and_immutable_resource() {
    let exact = pending_fixture();
    for change in 0..4 {
        let mut snapshot = exact.admission.snapshot.clone();
        let progress = snapshot.resources.get_mut(&exact.drain.resource).unwrap();
        match change {
            0 => progress.active_attempt = None,
            1 => progress.version += 1,
            2 => progress.removal = RemovalProgress::Retained,
            _ => snapshot.intent = fixture::intent(fixture::SOURCE, 2),
        }
        assert!(
            PendingInstanceDrainClosure::new(
                snapshot,
                exact.admission.receipt.command,
                exact.target.clone(),
                exact.drain.clone(),
            )
            .is_err()
        );
    }
    let mut recovery = exact.admission.snapshot.clone();
    recovery.lifecycle = DeploymentLifecycle::RecoveryRequired;
    recovery
        .resources
        .get_mut(&exact.drain.resource)
        .unwrap()
        .removal = RemovalProgress::RecoveryRequired;
    assert!(
        PendingInstanceDrainClosure::new(
            recovery,
            exact.admission.receipt.command,
            exact.target,
            exact.drain,
        )
        .is_ok()
    );
}

#[test]
fn pending_cannot_replace_claim_generation_provenance_or_original_actor() {
    let exact = pending_fixture();
    for change in 0..4 {
        let mut drain = exact.drain.clone();
        match change {
            0 => drain.generation = 0,
            1 => drain.provenance.actor_id = exact.admission.receipt.command.actor_id(),
            2 => drain.input_hash = ContentHash::digest(b"foreign plan"),
            _ => {
                drain.provenance.attempt_id =
                    DeploymentAttemptId::from_uuid(Uuid::from_u128(51)).unwrap();
            }
        }
        assert!(
            PendingInstanceDrainClosure::new(
                exact.admission.snapshot.clone(),
                exact.admission.receipt.command,
                exact.target.clone(),
                drain,
            )
            .is_err()
        );
    }
    assert!(
        PendingInstanceDrainClosure::new(
            exact.admission.snapshot,
            exact.drain.command,
            exact.target,
            exact.drain,
        )
        .is_err()
    );
}

#[test]
fn complete_pending_inventory_denies_orphans_duplicates_and_closed_conflicts() {
    let exact = pending_fixture();
    let active = continuation(&exact);
    for (continuations, pending, closures) in [
        (vec![], vec![exact.pending.clone()], vec![]),
        (vec![active.clone()], vec![], vec![]),
        (
            vec![active.clone(), active.clone()],
            vec![exact.pending.clone()],
            vec![],
        ),
        (
            vec![active.clone()],
            vec![exact.pending.clone(), exact.pending.clone()],
            vec![],
        ),
        (
            vec![active],
            vec![exact.pending.clone()],
            vec![closure(&exact)],
        ),
    ] {
        assert!(
            RemoveResumeAdmission::with_pending_instance_drains(
                exact.admission.clone(),
                continuations,
                closures,
                pending,
            )
            .is_err()
        );
    }
}

#[test]
fn pending_compares_whole_admitted_snapshot_and_current_command() {
    let exact = pending_fixture();
    for change in 0..3 {
        let mut admission = exact.admission.clone();
        match change {
            0 => admission.snapshot.version += 1,
            1 => {
                admission
                    .snapshot
                    .resources
                    .get_mut(&exact.drain.resource)
                    .unwrap()
                    .install = InstallProgress::Failed;
            }
            _ => admission.receipt.command = exact.drain.command,
        }
        assert!(
            RemoveResumeAdmission::with_pending_instance_drains(
                admission,
                vec![continuation(&exact)],
                vec![],
                vec![exact.pending.clone()],
            )
            .is_err()
        );
    }
}

#[test]
fn two_different_resources_cannot_share_an_original_active_attempt() {
    let exact = pending_fixture();
    let prepared = fixture::prepared().instances()[1].clone();
    let plan = &exact.admission.snapshot.intent.resources()[prepared.resource()];
    let original = EffectClaim {
        resource: prepared.resource().clone(),
        identity: plan.identity(),
        input_hash: plan.input_hash(),
        ..exact.target.original().clone()
    };
    let target = PreparedInstanceSafetyTarget::new(
        original.clone(),
        prepared,
        exact.target.provider_namespace().into(),
        exact.target.host_id().into(),
    )
    .unwrap();
    let other = EffectClaim {
        resource: original.resource,
        identity: original.identity,
        input_hash: original.input_hash,
        ..exact.drain.clone()
    };
    let mut admission = exact.admission.clone();
    let progress = admission.snapshot.resources[&exact.drain.resource].clone();
    admission
        .snapshot
        .resources
        .insert(other.resource.clone(), progress);
    let first = PendingInstanceDrainClosure::new(
        admission.snapshot.clone(),
        admission.receipt.command,
        exact.target,
        exact.drain.clone(),
    )
    .unwrap();
    let second = PendingInstanceDrainClosure::new(
        admission.snapshot.clone(),
        admission.receipt.command,
        target,
        other.clone(),
    )
    .unwrap();
    assert!(
        RemoveResumeAdmission::with_pending_instance_drains(
            admission,
            vec![
                RemoveContinuation {
                    claim: exact.drain,
                    closure: None
                },
                RemoveContinuation {
                    claim: other,
                    closure: None
                },
            ],
            vec![],
            vec![first, second],
        )
        .is_err()
    );
}

fn resumed(
    exact: &PendingFixture,
    version: u64,
) -> Result<ResumedPreparedInstanceClosureRequest, DeploymentError> {
    ResumedPreparedInstanceClosureRequest::new(
        exact.admission.receipt.command,
        exact.admission.snapshot.intent.clone(),
        exact.target.clone(),
        exact.drain.clone(),
        exact.pending.clone(),
        version,
    )
}

#[test]
fn resumed_publication_keeps_old_removal_identity_and_fresh_publisher_separate() {
    let exact = pending_fixture();
    let request = resumed(&exact, 7).unwrap();
    let original = closure(&exact);
    assert_eq!(request.removal_id(), original.removal_id);
    assert_eq!(request.command_key(), original.command_key);
    assert_eq!(request.drain(), &exact.drain);
    assert_ne!(
        request.current_command().actor_id(),
        request.drain().command.actor_id()
    );
    assert_eq!(request.expected_instance_version(), 7);
    assert_eq!(request.expected_deployment_version(), 9);
    assert!(resumed(&exact, 0).is_err());
    assert!(resumed(&exact, i64::MAX.cast_unsigned() + 1).is_err());
    assert!(
        ResumedPreparedInstanceClosureRequest::new(
            exact.drain.command,
            request.intent().clone(),
            request.target().clone(),
            exact.drain.clone(),
            exact.pending.clone(),
            7,
        )
        .is_err()
    );
    assert!(
        ResumedPreparedInstanceClosureRequest::new(
            request.current_command(),
            fixture::intent(fixture::SOURCE, 2),
            request.target().clone(),
            exact.drain.clone(),
            exact.pending.clone(),
            7,
        )
        .is_err()
    );
    let mut changed = exact.drain.clone();
    changed.generation += 1;
    assert!(
        ResumedPreparedInstanceClosureRequest::new(
            request.current_command(),
            request.intent().clone(),
            request.target().clone(),
            changed,
            exact.pending.clone(),
            7,
        )
        .is_err()
    );
    let target = PreparedInstanceSafetyTarget::new(
        exact.target.original().clone(),
        exact.target.prepared().clone(),
        "different-owner".into(),
        exact.target.host_id().into(),
    )
    .unwrap();
    assert!(
        ResumedPreparedInstanceClosureRequest::new(
            request.current_command(),
            request.intent().clone(),
            target,
            exact.drain,
            exact.pending,
            7,
        )
        .is_err()
    );
}

struct Unsupported;
#[async_trait]
impl PreparedInstanceClosureRepository for Unsupported {}

#[test]
fn unsupported_resumed_publication_returns_explicit_denial_without_fallback() {
    let exact = pending_fixture();
    let request = resumed(&exact, 7).unwrap();
    let current = fixture::identity(11, 12);
    let port = Unsupported;
    let mut future = Box::pin(port.close_resumed_for_remove(&current, &request));
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
}
