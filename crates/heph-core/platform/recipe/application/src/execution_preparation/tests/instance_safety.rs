use super::fixture;
use crate::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, EffectClaim,
    PreparedInstanceSafetyRepository, PreparedInstanceSafetyTarget, ResourceAction,
};
use async_trait::async_trait;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

fn target() -> PreparedInstanceSafetyTarget {
    let intent = fixture::intent(fixture::SOURCE, 1);
    let prepared = fixture::prepared().instances()[0].clone();
    let plan = &intent.resources()[prepared.resource()];
    let actor = fixture::identity(7, 8);
    let claim = EffectClaim {
        command: fixture::command(),
        deployment_id: intent.id(),
        resource: prepared.resource().clone(),
        identity: plan.identity(),
        input_hash: plan.input_hash(),
        resource_version: 1,
        generation: 1,
        action: ResourceAction::Create,
        provenance: AttemptProvenance::new(
            &actor,
            DeploymentAttemptId::from_uuid(uuid::Uuid::from_u128(20)).unwrap(),
        )
        .unwrap(),
    };
    PreparedInstanceSafetyTarget::new(
        claim,
        prepared,
        "actual-owner:opaque".into(),
        "host-a".into(),
    )
    .unwrap()
}
#[test]
fn original_instance_identity_and_cleanup_labels_are_checked() {
    let exact = target();
    assert_eq!(exact.provider_namespace(), "actual-owner:opaque");
    let mut wrong = exact.original().clone();
    wrong.resource = capability_domain::CapabilitySlotKey::parse("unrelated").unwrap();
    assert_ne!(wrong.resource, *exact.prepared().resource());
    assert!(
        PreparedInstanceSafetyTarget::new(
            wrong,
            exact.prepared().clone(),
            "actual-owner:opaque".into(),
            "host-a".into()
        )
        .is_err()
    );
    assert!(
        PreparedInstanceSafetyTarget::new(
            exact.original().clone(),
            exact.prepared().clone(),
            "../foreign".into(),
            "host-a".into()
        )
        .is_err()
    );
}
#[test]
fn target_cannot_promote_another_action_or_actor() {
    let exact = target();
    let mut changed = exact.original().clone();
    changed.action = ResourceAction::Detach;
    assert!(
        PreparedInstanceSafetyTarget::new(
            changed,
            exact.prepared().clone(),
            "actual-owner:opaque".into(),
            "host-a".into()
        )
        .is_err()
    );
    let mut changed = exact.original().clone();
    changed.provenance.actor_id = identity_domain::UserId::new();
    assert!(
        PreparedInstanceSafetyTarget::new(
            changed,
            exact.prepared().clone(),
            "actual-owner:opaque".into(),
            "host-a".into()
        )
        .is_err()
    );
}
struct Unsupported;
#[async_trait]
impl PreparedInstanceSafetyRepository for Unsupported {}
#[test]
fn unsupported_observer_never_falls_back_to_original_import() {
    let exact = target();
    let port = Unsupported;
    let mut future = Box::pin(port.observe_safety(&exact));
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
}
