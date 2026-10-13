//! Controlled real Core state/input comparisons; no `PostgreSQL` or physical proof.
use super::{
    Mode, Request,
    retained::{Final, Original},
    validation::needs_drain,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentId, DeploymentOperation, RemovalProgress, RemoveDeployment,
    ResourceAction,
};
use uuid::Uuid;
fn request(actor: u128, logical: u128, expected: u64, mode: Mode) -> Request {
    let identity = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(actor)),
        "controlled",
        "actor",
        serde_json::json!({}),
        RequestId::from_uuid(Uuid::from_u128(logical)),
    );
    let command = CommandIdentity::from_identity(&identity, DeploymentOperation::Remove).unwrap();
    Request {
        identity,
        remove: RemoveDeployment {
            command,
            deployment_id: DeploymentId::from_uuid(Uuid::from_u128(42)).unwrap(),
            expected_version: expected,
        },
        mode,
    }
}
#[test]
fn terminal_effects_never_rerun_drain_and_unknowns_never_classify() {
    assert!(
        needs_drain(
            RemovalProgress::RecoveryRequired,
            Some(ResourceAction::Drain)
        )
        .unwrap()
    );
    assert!(
        !needs_drain(
            RemovalProgress::RecoveryRequired,
            Some(ResourceAction::Detach)
        )
        .unwrap()
    );
    assert!(!needs_drain(RemovalProgress::Draining, Some(ResourceAction::Delete)).unwrap());
    assert!(!needs_drain(RemovalProgress::Deleted, None).unwrap()); // Terminal still positively qualifies Applied history.
    assert!(needs_drain(RemovalProgress::RecoveryRequired, None).is_err());
    assert!(needs_drain(RemovalProgress::Pending, Some(ResourceAction::Create)).is_err());
    assert!(needs_drain(RemovalProgress::Pending, None).unwrap());
}
#[test]
fn saved_original_input_cannot_be_replaced_with_current_progress_cas() {
    let first = request(1, 2, 12, Mode::Original);
    let mut original = Original::new(first.clone());
    let mut changed = first;
    changed.remove.expected_version = 19;
    assert!(original.request(&changed).is_err());
    assert_eq!(original.requests[0].remove.expected_version, 12);
    assert_eq!(original.requests.len(), 1);
}
#[test]
fn current_manager_is_distinct_from_saved_actor_not_rebuilt_credentials() {
    let first = request(1, 2, 12, Mode::Original);
    let mut original = Original::new(first.clone());
    let current = request(3, 4, 19, Mode::Resume);
    original.request(&current).unwrap();
    current
        .remove
        .command
        .validate(&current.identity, DeploymentOperation::Remove)
        .unwrap();
    assert!(
        first
            .remove
            .command
            .validate(&current.identity, DeploymentOperation::Remove)
            .is_err()
    );
    assert_eq!(original.requests[0].identity, first.identity);
    assert_eq!(original.requests[1].identity, current.identity);
}
#[test]
fn uncertain_finish_keeps_original_before_cas_and_each_manager_separate() {
    let first = request(1, 2, 12, Mode::Original);
    let mut original = Original::new(first.clone());
    original.finalizations.push(Final {
        command: first.remove.command,
        before: 27,
        receipt: None,
    });
    let current = request(3, 4, 28, Mode::Resume);
    original.request(&current).unwrap();
    original.finalizations.push(Final {
        command: current.remove.command,
        before: 28,
        receipt: None,
    });
    assert_eq!(original.finalizations[0].before, 27);
    assert_ne!(
        original.finalizations[0].command,
        original.finalizations[1].command
    );
    assert!(original.finalizations[0].receipt.is_none()); // Unknown is not rolled back or replaced.
}
