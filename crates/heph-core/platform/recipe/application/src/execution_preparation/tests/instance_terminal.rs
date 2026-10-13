//! Controlled comparison tests using checked planning/import fixture helpers.
//! Manually assembled historical claims/closures are not PG or physical proof.
mod fixture;
#[path = "instance_terminal/preparation.rs"]
mod preparation_tests;
use super::fixture as preparation;
use crate::{
    DeploymentError, PreparedInstanceDeleteObservation, PreparedInstanceDetachObservation,
    PreparedInstanceSafetyState, PreparedInstanceSafetyTarget, PreparedInstanceTerminalRepository,
    RemoveResumeConfiguration, ResourceAction,
};
use async_trait::async_trait;
use fixture::{TerminalFixture, attempt, fresh_command, safety};
use identity_domain::AuthenticatedIdentity;
use release_domain::{ContentHash, InstanceRemovalId};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};
use uuid::Uuid;

#[test]
fn requests_preserve_whole_originals_and_separate_current_publisher() {
    let mut exact = TerminalFixture::new(ResourceAction::Detach);
    let original = exact.detach().unwrap();
    assert_eq!(original.current_command(), exact.effect.command);
    exact.current = fresh_command();
    let fresh = exact.detach().unwrap();
    assert_eq!(fresh.intent(), &exact.intent);
    assert_eq!(fresh.target(), &exact.target);
    assert_eq!(fresh.closure(), &exact.closure);
    assert_eq!(fresh.effect(), &exact.effect);
    assert_eq!(fresh.configuration(), &exact.configuration);
    assert_eq!(fresh.current_command(), exact.current);
    assert_ne!(fresh.effect().command, fresh.current_command());
    assert_ne!(fresh.closure().actor, fresh.current_command().actor_id());
    assert_ne!(fresh, original);
    let mut exact = TerminalFixture::new(ResourceAction::Delete);
    exact.current = fresh_command();
    let delete = exact.delete(8).unwrap();
    assert_eq!(delete.intent(), &exact.intent);
    assert_eq!(delete.target(), &exact.target);
    assert_eq!(delete.closure(), &exact.closure);
    assert_eq!(delete.effect(), &exact.effect);
    assert_eq!(delete.configuration(), &exact.configuration);
    assert_eq!(delete.current_command(), exact.current);
    assert_eq!(delete.expected_instance_version(), 8);
}

#[test]
fn action_full_claim_provenance_and_attempt_reuse_are_rejected() {
    for change in 0..12 {
        let mut exact = TerminalFixture::new(ResourceAction::Detach);
        match change {
            0 => exact.effect.action = ResourceAction::Delete,
            1 => exact.effect.command = preparation::command(),
            2 => exact.effect.provenance.actor_id = preparation::identity(13, 14).user_id,
            3 => {
                exact.effect.provenance.request_id =
                    identity_domain::RequestId::from_uuid(Uuid::nil());
            }
            4 => exact.effect.generation = 0,
            5 => exact.effect.resource_version = 0,
            6 => exact.effect.provenance.attempt_id = exact.closure.drain.provenance.attempt_id,
            7 => exact.effect.provenance.attempt_id = exact.target.original().provenance.attempt_id,
            8 => exact.effect.input_hash = ContentHash::digest(b"changed full input"),
            9 => {
                exact.effect.resource =
                    capability_domain::CapabilitySlotKey::parse("other").unwrap();
            }
            10 => {
                exact.effect.deployment_id =
                    crate::DeploymentId::from_uuid(Uuid::from_u128(80)).unwrap();
            }
            _ => exact.current = preparation::command(),
        }
        assert!(exact.detach().is_err(), "substitution {change}");
    }
    assert!(
        TerminalFixture::new(ResourceAction::Detach)
            .delete(8)
            .is_err()
    );
    assert!(
        TerminalFixture::new(ResourceAction::Delete)
            .detach()
            .is_err()
    );
    let retain = TerminalFixture::for_intent(
        preparation::intent(preparation::SOURCE, 1),
        ResourceAction::Delete,
    );
    assert!(matches!(
        retain.delete(8),
        Err(DeploymentError::InvalidAction)
    ));
}

#[test]
fn immutable_intent_target_and_configuration_substitutions_deny() {
    for change in 0..5 {
        let mut exact = TerminalFixture::new(ResourceAction::Detach);
        match change {
            0 => exact.intent = preparation::intent(preparation::SOURCE, 2),
            1 => {
                let mut catalog = preparation::catalog();
                catalog.parameters[0].default =
                    Some(release_domain::ParameterValue::String("different".into()));
                let source = preparation::SOURCE.replace(
                    "kind = \"instance\"",
                    "kind = \"instance\"\nremoval = \"delete\"",
                );
                exact.intent = preparation::with_catalog(
                    &source,
                    1,
                    catalog,
                    &std::collections::BTreeMap::new(),
                );
            }
            _ => {
                let namespace = if change == 2 {
                    "foreign"
                } else {
                    exact.target.provider_namespace()
                };
                let host = if change == 3 {
                    "foreign"
                } else {
                    exact.target.host_id()
                };
                let platform = if change == 4 {
                    preparation::platform("changed")
                } else {
                    preparation::prepared().platform().clone()
                };
                exact.configuration = RemoveResumeConfiguration::new(
                    namespace.into(),
                    host.into(),
                    platform,
                    preparation::prepared().profile(),
                )
                .unwrap();
            }
        }
        assert!(exact.detach().is_err(), "substitution {change}");
    }
    let mut exact = TerminalFixture::new(ResourceAction::Delete);
    let mut original = exact.target.original().clone();
    original.input_hash = ContentHash::digest(b"substituted original hash");
    exact.target = PreparedInstanceSafetyTarget::new(
        original,
        exact.target.prepared().clone(),
        exact.target.provider_namespace().into(),
        exact.target.host_id().into(),
    )
    .unwrap();
    assert!(exact.delete(8).is_err());
}

#[test]
fn strict_original_closure_is_not_relaxed_for_a_fresh_manager() {
    for change in 0..6 {
        let mut exact = TerminalFixture::new(ResourceAction::Delete);
        exact.current = fresh_command();
        match change {
            0 => exact.closure.actor = exact.current.actor_id(),
            1 => exact.closure.drain.provenance.attempt_id = attempt(70),
            2 => exact.closure.removal_id = InstanceRemovalId::from_uuid(Uuid::from_u128(70)),
            3 => exact.closure.expected_version = 0,
            4 => exact.closure.request = identity_domain::RequestId::from_uuid(Uuid::nil()),
            _ => exact.closure.drain.action = ResourceAction::Delete,
        }
        assert!(exact.delete(8).is_err(), "substitution {change}");
    }
}

#[test]
fn detach_observation_binds_whole_target_removal_and_retains_full_inventory() {
    let exact = TerminalFixture::new(ResourceAction::Detach);
    let observed = safety(&exact);
    let bound =
        PreparedInstanceDetachObservation::new(exact.detach().unwrap(), observed.clone()).unwrap();
    assert_eq!(bound.request(), &exact.detach().unwrap());
    assert_eq!(bound.safety(), &observed);
    for state in [
        PreparedInstanceSafetyState::Unproven,
        PreparedInstanceSafetyState::Created,
        PreparedInstanceSafetyState::ClosedHeld,
    ] {
        let mut wrong = observed.clone();
        wrong.state = state;
        assert!(PreparedInstanceDetachObservation::new(exact.detach().unwrap(), wrong).is_err());
    }
    let mut wrong = observed.clone();
    wrong.removal_id = Some(InstanceRemovalId::from_uuid(Uuid::from_u128(86)));
    assert!(PreparedInstanceDetachObservation::new(exact.detach().unwrap(), wrong).is_err());
    let mut wrong = observed;
    wrong.target = PreparedInstanceSafetyTarget::new(
        exact.target.original().clone(),
        exact.target.prepared().clone(),
        "foreign".into(),
        exact.target.host_id().into(),
    )
    .unwrap();
    assert!(PreparedInstanceDetachObservation::new(exact.detach().unwrap(), wrong).is_err());
    let source = crate::SourceObservation::new(
        preparation::pin(),
        ContentHash::digest(b"different immutable authored contract"),
        &format!("registry.example/sqlite@sha256:{}", "b".repeat(64)),
        preparation::policy(128),
        exact.configuration.platform().clone(),
    )
    .unwrap();
    let prepared = crate::DeploymentExecutionPreparation::new(
        &exact.intent,
        preparation::command(),
        exact.configuration.profile(),
        exact.configuration.platform().clone(),
        &[source],
    )
    .unwrap()
    .instances()[0]
        .clone();
    let mut wrong = safety(&exact);
    wrong.target = PreparedInstanceSafetyTarget::new(
        exact.target.original().clone(),
        prepared,
        exact.target.provider_namespace().into(),
        exact.target.host_id().into(),
    )
    .unwrap();
    assert!(PreparedInstanceDetachObservation::new(exact.detach().unwrap(), wrong).is_err());
}

#[test]
fn delete_requires_explicit_bounded_cas_and_exact_successor_record() {
    let exact = TerminalFixture::new(ResourceAction::Delete);
    for version in [0, i64::MAX.cast_unsigned() + 1] {
        assert!(exact.delete(version).is_err());
    }
    let request = exact.delete(8).unwrap();
    let record = Uuid::from_u128(87);
    let observed = PreparedInstanceDeleteObservation::new(request.clone(), record, 9).unwrap();
    assert_eq!(observed.request(), &request);
    assert_eq!(observed.terminal_operation_id(), record);
    assert_eq!(observed.after_instance_version(), 9);
    for version in [0, 8, 10, i64::MAX.cast_unsigned() + 1] {
        assert!(PreparedInstanceDeleteObservation::new(request.clone(), record, version).is_err());
    }
    assert!(PreparedInstanceDeleteObservation::new(request, Uuid::nil(), 9).is_err());
    let maximum = exact.delete(i64::MAX.cast_unsigned()).unwrap();
    assert!(
        PreparedInstanceDeleteObservation::new(maximum, record, i64::MAX.cast_unsigned()).is_err()
    );
}

struct Unsupported;
#[async_trait]
impl PreparedInstanceTerminalRepository for Unsupported {}
fn denied<T>(future: impl Future<Output = Result<T, DeploymentError>>) {
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
}
#[test]
fn unsupported_ports_deny_without_effect_or_observation_fallback() {
    let port = Unsupported;
    let detach = TerminalFixture::new(ResourceAction::Detach)
        .detach()
        .unwrap();
    let delete = TerminalFixture::new(ResourceAction::Delete)
        .delete(8)
        .unwrap();
    let identity: AuthenticatedIdentity = preparation::identity(11, 12);
    denied(port.observe_detach(&detach));
    denied(port.delete_for_remove(&identity, &delete));
    denied(port.observe_deleted(&delete));
}
