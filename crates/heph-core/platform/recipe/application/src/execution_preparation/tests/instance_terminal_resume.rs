//! Controlled contract denials; not native history/authority or physical evidence.
mod fixture;
mod inventory;
use self::fixture::{exact, progress};
use super::instance_delete_fixture as original;
use crate::{
    DeploymentError, PreparedInstanceDeleteCompletedHistory, PreparedInstanceDeleteContinuation,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteHistoryState,
    PreparedInstanceDeletePublication, PreparedInstanceDetachRecovery,
    PreparedInstanceTerminalHistoryRepository, PreparedInstanceTerminalProgress,
    PreparedInstanceTerminalPublisher, RemovalProgress, ResourceAction,
};
use async_trait::async_trait;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};
use uuid::Uuid;

#[test]
fn detach_recovery_retains_exact_originals_and_rejects_progress_substitutions() {
    let exact = exact(ResourceAction::Detach);
    let request = fixture::detach(&exact);
    let observed = progress(&exact.effect);
    let recovery = PreparedInstanceDetachRecovery::new(request.clone(), observed.clone()).unwrap();
    assert_eq!(recovery.request(), &request);
    assert_eq!(recovery.observed_progress(), &observed);
    for change in 0..5 {
        let mut state = observed.progress().clone();
        let mut generation = observed.generation();
        let mut prior = observed.last_action();
        match change {
            0 => state.active_attempt = None,
            1 => generation += 1,
            2 => state.version = exact.effect.resource_version,
            3 => state.diagnostic = None,
            _ => prior = Some(ResourceAction::Detach),
        }
        let wrong = PreparedInstanceTerminalProgress::new(state, generation, prior).unwrap();
        assert!(PreparedInstanceDetachRecovery::new(request.clone(), wrong).is_err());
    }
    let mut wrong = exact;
    wrong.effect.generation += 1;
    assert!(
        PreparedInstanceDetachRecovery::new(fixture::detach(&wrong), progress(&wrong.effect))
            .is_err()
    );
}

#[test]
fn nullable_terminal_tuple_is_atomic_and_inactive_requires_committed_applied() {
    let exact = exact(ResourceAction::Delete);
    let request = fixture::delete(&exact);
    for mask in 0..8 {
        let observed = PreparedInstanceDeleteHistory::from_observed_parts(
            request.clone(),
            progress(&exact.effect),
            (mask & 1 != 0).then_some(Uuid::from_u128(90)),
            (mask & 2 != 0).then_some(9),
            (mask & 4 != 0).then(|| fixture::publisher(&exact)),
        );
        assert_eq!(observed.is_ok(), mask == 0 || mask == 7);
    }
    assert!(
        PreparedInstanceDeleteCompletedHistory::new(
            PreparedInstanceDeleteContinuation::unwritten(request.clone()),
            fixture::applied(&exact.effect),
            None
        )
        .is_err()
    );
    assert!(
        PreparedInstanceDeleteHistory::from_observed_parts(
            request,
            fixture::applied(&exact.effect),
            None,
            None,
            None
        )
        .is_err()
    );
    let completed = fixture::committed(&exact, fixture::applied(&exact.effect)).unwrap();
    assert_eq!(
        completed.state(),
        PreparedInstanceDeleteHistoryState::Applied
    );
    assert!(
        completed
            .observed_progress()
            .progress()
            .active_attempt
            .is_none()
    );
}

#[test]
fn saved_publisher_and_authoritative_before_cas_survive_another_current_manager() {
    let exact = exact(ResourceAction::Delete);
    let history = fixture::committed(&exact, progress(&exact.effect)).unwrap();
    assert_eq!(history.state(), PreparedInstanceDeleteHistoryState::Active);
    let PreparedInstanceDeletePublication::Committed(saved) = history.original().publication()
    else {
        panic!("original publication was lost")
    };
    assert_ne!(
        saved.publisher(),
        history.original().original().current_command()
    );
    assert_ne!(saved.publisher(), exact.effect.command);
    assert_eq!(saved.before_instance_version(), 8);
    assert_eq!(saved.after_instance_version(), 9);
    assert_eq!(
        history.publisher_admission_command(),
        Some(saved.native_command_id())
    );
    let current = original::fresh_command();
    let rebuilt = history.original().rebuild_for_remove(current).unwrap();
    assert_eq!(rebuilt.expected_instance_version(), 8);
    assert_eq!(rebuilt.effect(), &exact.effect);
    assert_eq!(rebuilt.closure(), &exact.closure);
    assert_eq!(rebuilt.configuration(), &exact.configuration);
}

#[test]
fn publisher_nullable_admission_is_exact_original_vs_fresh_not_current_actor() {
    let exact = exact(ResourceAction::Delete);
    let original = exact.effect.command;
    PreparedInstanceTerminalPublisher::new(
        &exact.effect,
        original,
        original.id(),
        original.actor_id(),
        exact.effect.provenance.request_id,
        None,
    )
    .unwrap();
    assert!(
        PreparedInstanceTerminalPublisher::new(
            &exact.effect,
            original,
            original.id(),
            original.actor_id(),
            exact.effect.provenance.request_id,
            Some(original.id())
        )
        .is_err()
    );
    let saved = fixture::publisher(&exact);
    for (id, actor, admission) in [
        (saved.command_id(), saved.actor_id(), None),
        (original.id(), saved.actor_id(), Some(saved.command_id())),
        (
            saved.command_id(),
            original.actor_id(),
            Some(saved.command_id()),
        ),
    ] {
        assert!(
            PreparedInstanceTerminalPublisher::new(
                &exact.effect,
                saved.command(),
                id,
                actor,
                saved.request_id(),
                admission
            )
            .is_err()
        );
    }
}

#[test]
fn original_generation_gaps_cas_overflow_and_changed_endpoint_deny() {
    let mut exact = exact(ResourceAction::Delete);
    for after in [8, 10, i64::MAX.cast_unsigned()] {
        assert!(
            PreparedInstanceDeleteHistory::from_observed_parts(
                fixture::delete(&exact),
                progress(&exact.effect),
                Some(Uuid::from_u128(90)),
                Some(after),
                Some(fixture::publisher(&exact))
            )
            .is_err()
        );
    }
    let maximum = exact
        .request_for(exact.current, i64::MAX.cast_unsigned())
        .unwrap();
    assert!(
        PreparedInstanceDeleteHistory::from_observed_parts(
            maximum,
            progress(&exact.effect),
            None,
            None,
            None
        )
        .is_err()
    );
    assert!(
        PreparedInstanceDeleteHistory::from_observed_parts(
            fixture::delete(&exact),
            progress(&exact.effect),
            Some(Uuid::nil()),
            Some(9),
            Some(fixture::publisher(&exact))
        )
        .is_err()
    );
    let mut state = progress(&exact.effect).progress().clone();
    state.removal = RemovalProgress::Deleted;
    let substituted = PreparedInstanceTerminalProgress::new(
        state,
        exact.effect.generation,
        Some(ResourceAction::Delete),
    )
    .unwrap();
    assert!(fixture::committed(&exact, substituted).is_err());
    exact.effect.generation += 1;
    assert!(fixture::committed(&exact, progress(&exact.effect)).is_err());
}

struct Unsupported;
#[async_trait]
impl PreparedInstanceTerminalHistoryRepository for Unsupported {}
#[test]
fn optional_history_ports_default_deny_without_absence_or_applied_fallback() {
    let exact = exact(ResourceAction::Delete);
    let request = fixture::delete(&exact);
    let actor = super::fixture::identity(15, 16);
    let adapter = Unsupported;
    let mut context = Context::from_waker(Waker::noop());
    let mut active = Box::pin(adapter.load_delete_continuation(&actor, &request));
    assert!(matches!(
        active.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
    let mut original = Box::pin(adapter.load_original_delete_history(
        &actor,
        request.intent(),
        request.current_command(),
        &request.effect().resource,
    ));
    assert!(matches!(
        original.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
    let mut completed = Box::pin(adapter.observe_completed_delete(&request));
    assert!(matches!(
        completed.as_mut().poll(&mut context),
        Poll::Ready(Err(DeploymentError::InvalidAction))
    ));
}
