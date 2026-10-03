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
