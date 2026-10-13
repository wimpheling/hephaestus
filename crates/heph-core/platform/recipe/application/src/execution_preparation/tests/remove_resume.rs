use super::{fixture, instance_safety};
use crate::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation,
    OriginalInstanceClosure, PreparedInstanceClosureRequest, RemoveResumeConfiguration,
    ResourceAction,
};
use release_domain::ContentHash;
use uuid::Uuid;
fn original() -> (OriginalInstanceClosure, crate::EffectClaim) {
    let target = instance_safety::target();
    let manager = fixture::identity(9, 10);
    let claim = crate::EffectClaim {
        command: CommandIdentity::from_identity(&manager, DeploymentOperation::Remove).unwrap(),
        action: ResourceAction::Drain,
        generation: 2,
        resource_version: 3,
        provenance: AttemptProvenance::new(
            &manager,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(50)).unwrap(),
        )
        .unwrap(),
        ..target.original().clone()
    };
    let request = PreparedInstanceClosureRequest::new(
        fixture::intent(fixture::SOURCE, 1),
        target,
        claim.clone(),
        1,
    )
    .unwrap();
    (
        OriginalInstanceClosure {
            publication_origin: crate::InstanceClosurePublicationOrigin::OriginalDrain,
            drain: claim.clone(),
            removal_id: request.removal_id(),
            command_key: request.command_key(),
            expected_version: 1,
            actor: manager.user_id,
            request: manager.request_id,
            input_hash: ContentHash::digest(b"exact original comparison"),
        },
        claim,
    )
}
#[test]
fn original_closure_survives_new_actor_and_later_action_without_new_identity() {
    let (closure, mut continuation) = original();
    closure.validate(&continuation).unwrap();
    let fresh = fixture::identity(11, 12);
    continuation.command =
        CommandIdentity::from_identity(&fresh, DeploymentOperation::Remove).unwrap();
    continuation.provenance = AttemptProvenance::new(
        &fresh,
        DeploymentAttemptId::from_uuid(Uuid::from_u128(51)).unwrap(),
    )
    .unwrap();
    continuation.action = ResourceAction::Detach;
    closure.validate(&continuation).unwrap();
    assert_ne!(closure.actor, continuation.command.actor_id());
    assert_ne!(
        closure.drain.provenance.attempt_id,
        continuation.provenance.attempt_id
    );
}
#[test]
fn original_closure_rejects_substituted_drain_or_intent() {
    let (closure, continuation) = original();
    let mut changed = closure.clone();
    changed.drain.provenance.attempt_id =
        DeploymentAttemptId::from_uuid(Uuid::from_u128(52)).unwrap();
    assert!(changed.validate(&continuation).is_err());
    let mut changed = continuation;
    changed.input_hash = ContentHash::digest(b"different immutable intent");
    assert!(closure.validate(&changed).is_err());
}
#[test]
fn actual_opaque_provider_configuration_is_checked_comparison_data() {
    let prepared = fixture::prepared();
    let exact = RemoveResumeConfiguration::new(
        "opaque-owner:managed".into(),
        "host-a".into(),
        prepared.platform().clone(),
        prepared.profile(),
    )
    .unwrap();
    assert_eq!(exact.namespace(), "opaque-owner:managed");
    assert!(
        RemoveResumeConfiguration::new(
            "../foreign".into(),
            "host-a".into(),
            prepared.platform().clone(),
            prepared.profile()
        )
        .is_err()
    );
}

fn admission() -> crate::DeploymentAdmission {
    let intent = fixture::intent(fixture::SOURCE, 1);
    let current = fixture::identity(11, 12);
    let command = CommandIdentity::from_identity(&current, DeploymentOperation::Remove).unwrap();
    crate::DeploymentAdmission {
        disposition: crate::AdmissionDisposition::Resume,
        snapshot: crate::DeploymentSnapshot {
            intent: intent.clone(),
            lifecycle: crate::DeploymentLifecycle::Removing,
            version: 9,
            resources: std::collections::BTreeMap::new(),
        },
        receipt: crate::CommandReceipt {
            command,
            deployment_id: intent.id(),
            input_hash: ContentHash::digest(b"controlled admission"),
            lifecycle: crate::DeploymentLifecycle::Removing,
            deployment_version: 9,
            event: event_application::CommittedMutation {
                event_id: Uuid::from_u128(90),
                scope_kind: "project".into(),
                scope_id: intent.project_id().as_uuid(),
                cursor: 1,
                aggregate_version: 1,
            },
        },
    }
}

#[test]
fn inactive_original_closure_history_is_not_an_active_continuation_or_cleanup_proof() {
    let (closure, _) = original();
    let history = crate::RemoveResumeAdmission::with_complete_closures(
        admission(),
        vec![],
        vec![closure.clone()],
    )
    .unwrap();
    assert!(history.continuations().is_empty());
    assert_eq!(history.closures(), &[closure]);
}

#[test]
fn complete_history_rejects_duplicate_foreign_and_missing_active_closures() {
    let (closure, claim) = original();
    assert!(
        crate::RemoveResumeAdmission::with_complete_closures(
            admission(),
            vec![],
            vec![closure.clone(), closure.clone()],
        )
        .is_err()
    );
    let mut foreign = closure.clone();
    foreign.drain.deployment_id = crate::DeploymentId::from_uuid(Uuid::from_u128(99)).unwrap();
    assert!(
        crate::RemoveResumeAdmission::with_complete_closures(admission(), vec![], vec![foreign],)
            .is_err()
    );
    let active = crate::RemoveContinuation {
        claim,
        closure: Some(closure.clone()),
    };
    assert!(
        crate::RemoveResumeAdmission::with_complete_closures(
            admission(),
            vec![active.clone()],
            vec![],
        )
        .is_err()
    );
    let compatible = crate::RemoveResumeAdmission::new(admission(), vec![active]).unwrap();
    assert_eq!(compatible.closures(), &[closure]);
}
