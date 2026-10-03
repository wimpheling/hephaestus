use super::{fixture, instance_safety};
use crate::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation, EffectClaim,
    ExistingInstanceClosureRequest, OriginalInstanceClosure, PreparedInstanceClosureRequest,
    ResourceAction,
};
use release_domain::ContentHash;
use uuid::Uuid;
fn request() -> ExistingInstanceClosureRequest {
    let target = instance_safety::target();
    let actor = fixture::identity(31, 32);
    let drain = EffectClaim {
        command: CommandIdentity::from_identity(&actor, DeploymentOperation::Remove).unwrap(),
        action: ResourceAction::Drain,
        generation: 2,
        resource_version: 3,
        provenance: AttemptProvenance::new(
            &actor,
            DeploymentAttemptId::from_uuid(Uuid::from_u128(70)).unwrap(),
        )
        .unwrap(),
        ..target.original().clone()
    };
    let intent = fixture::intent(fixture::SOURCE, 1);
    let closed =
        PreparedInstanceClosureRequest::new(intent.clone(), target.clone(), drain.clone(), 1)
            .unwrap();
    let expected = OriginalInstanceClosure {
        drain,
        removal_id: closed.removal_id(),
        command_key: closed.command_key(),
        expected_version: 1,
        actor: actor.user_id,
        request: actor.request_id,
        input_hash: ContentHash::digest(b"exact closure fingerprint"),
    };
    let fresh = fixture::identity(33, 34);
    ExistingInstanceClosureRequest::new(
        CommandIdentity::from_identity(&fresh, DeploymentOperation::Remove).unwrap(),
        intent,
        target,
        expected,
    )
    .unwrap()
}
#[test]
fn current_remove_keeps_original_stable_closure_and_historical_actor_as_data() {
    let request = request();
    assert_ne!(request.command().actor_id(), request.expected().actor);
    assert_ne!(request.command(), request.expected().drain.command);
    assert_eq!(
        request.target().original().identity,
        request.expected().drain.identity
    );
}
#[test]
fn old_command_and_substituted_original_pins_are_rejected() {
    let request = request();
    assert!(
        ExistingInstanceClosureRequest::new(
            request.expected().drain.command,
            request.intent().clone(),
            request.target().clone(),
            request.expected().clone()
        )
        .is_err()
    );
    let mut changed = request.expected().clone();
    changed.drain.input_hash = ContentHash::digest(b"different input");
    assert!(
        ExistingInstanceClosureRequest::new(
            request.command(),
            request.intent().clone(),
            request.target().clone(),
            changed
        )
        .is_err()
    );
}
