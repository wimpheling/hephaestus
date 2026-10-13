use super::{fixture, instance_safety};
use crate::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentOperation,
    EffectClaim, PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyRepository, ResourceAction,
};
use async_trait::async_trait;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

fn request() -> PreparedInstanceClosureRequest {
    let target = instance_safety::target();
    let manager = fixture::identity(9, 10);
    let drain = EffectClaim {
        command: CommandIdentity::from_identity(&manager, DeploymentOperation::Remove).unwrap(),
        action: ResourceAction::Drain,
        provenance: AttemptProvenance::new(
            &manager,
            DeploymentAttemptId::from_uuid(uuid::Uuid::from_u128(30)).unwrap(),
        )
        .unwrap(),
        resource_version: 3,
        generation: 2,
        ..target.original().clone()
    };
    PreparedInstanceClosureRequest::new(fixture::intent(fixture::SOURCE, 1), target, drain, 1)
        .unwrap()
}
#[test]
fn stable_closure_identity_binds_original_instance_and_current_drain() {
    let exact = request();
    let same = PreparedInstanceClosureRequest::new(
        exact.intent().clone(),
        exact.target().clone(),
        exact.drain().clone(),
        1,
    )
    .unwrap();
    assert_eq!(same, exact);
    let mut other = exact.drain().clone();
    other.provenance.attempt_id =
        DeploymentAttemptId::from_uuid(uuid::Uuid::from_u128(31)).unwrap();
    let other = PreparedInstanceClosureRequest::new(
        exact.intent().clone(),
        exact.target().clone(),
        other,
        1,
    )
    .unwrap();
    assert_ne!(other.command_key(), exact.command_key());
    assert_ne!(other.removal_id(), exact.removal_id());
}
#[test]
fn closure_shape_rejects_another_resource_operation_and_zero_version() {
    let exact = request();
    let mut wrong = exact.drain().clone();
    wrong.resource = capability_domain::CapabilitySlotKey::parse("unrelated").unwrap();
    assert!(
        PreparedInstanceClosureRequest::new(
            exact.intent().clone(),
            exact.target().clone(),
            wrong,
            1
        )
        .is_err()
    );
    let mut wrong = exact.drain().clone();
    wrong.action = ResourceAction::Detach;
    assert!(
        PreparedInstanceClosureRequest::new(
            exact.intent().clone(),
            exact.target().clone(),
            wrong,
            1
        )
        .is_err()
    );
    assert!(
        PreparedInstanceClosureRequest::new(
            exact.intent().clone(),
            exact.target().clone(),
            exact.drain().clone(),
            0
        )
        .is_err()
    );
}
struct Unsupported;
#[async_trait]
impl PreparedInstanceClosureRepository for Unsupported {}
#[async_trait]
impl PreparedInstanceSafetyRepository for Unsupported {}
#[test]
fn unsupported_loader_and_closure_never_fall_back_to_import_or_removal() {
    let exact = request();
    let manager = fixture::identity(9, 10);
    let port = Unsupported;
    let mut context = Context::from_waker(Waker::noop());
    let mut load = Box::pin(port.load_original_target(
        &manager,
        exact.drain().command,
        exact.intent(),
        &exact.drain().resource,
    ));
    assert!(matches!(
        load.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
    let mut close = Box::pin(port.close_for_remove(&manager, &exact));
    assert!(matches!(
        close.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
}
