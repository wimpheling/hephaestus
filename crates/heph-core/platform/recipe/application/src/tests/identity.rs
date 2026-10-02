use std::str::FromStr;

use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use uuid::Uuid;

use crate::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentId,
    DeploymentKey, DeploymentLifecycle, DeploymentOperation, DiagnosticCode, InstallProgress,
    RemovalProgress,
};

#[test]
fn logical_command_identity_ignores_request_attempt_and_binds_actor_operation_seed() {
    let actor = UserId::from_uuid(Uuid::from_u128(1));
    let identity = AuthenticatedIdentity::new(
        actor,
        "test",
        "actor",
        json!({}),
        RequestId::from_uuid(Uuid::from_u128(2)),
    )
    .with_idempotency_id(RequestId::from_uuid(Uuid::from_u128(3)));
    let command =
        CommandIdentity::from_identity(&identity, DeploymentOperation::Install).expect("command");
    let mut retry = identity.clone();
    retry.request_id = RequestId::from_uuid(Uuid::from_u128(4));
    assert_eq!(
        command,
        CommandIdentity::from_identity(&retry, DeploymentOperation::Install).expect("retry")
    );
    let first = AttemptProvenance::new(
        &identity,
        DeploymentAttemptId::from_uuid(Uuid::from_u128(5)).expect("attempt"),
    )
    .expect("provenance");
    let second = AttemptProvenance::new(
        &retry,
        DeploymentAttemptId::from_uuid(Uuid::from_u128(6)).expect("attempt"),
    )
    .expect("retry provenance");
    assert_ne!(first.request_id, second.request_id);
    assert_ne!(first.attempt_id, second.attempt_id);
    command
        .validate(&retry, DeploymentOperation::Install)
        .expect("same logical command");
    assert!(
        command
            .validate(&retry, DeploymentOperation::Remove)
            .is_err()
    );
    retry.user_id = UserId::from_uuid(Uuid::from_u128(7));
    assert!(
        command
            .validate(&retry, DeploymentOperation::Install)
            .is_err()
    );
    retry = identity;
    retry.idempotency_id = RequestId::from_uuid(Uuid::from_u128(8));
    assert!(
        command
            .validate(&retry, DeploymentOperation::Install)
            .is_err()
    );
}

#[test]
fn identities_keys_and_unknown_persisted_states_are_bounded_and_checked() {
    assert!(DeploymentId::from_uuid(Uuid::nil()).is_err());
    assert!(DeploymentAttemptId::from_str("invalid").is_err());
    assert!(DeploymentKey::parse("é".repeat(64)).is_ok());
    for key in [
        String::new(),
        "x".repeat(129),
        "é".repeat(65),
        " padded ".to_owned(),
        "new\nline".to_owned(),
    ] {
        assert!(DeploymentKey::parse(key).is_err());
    }
    assert_eq!(
        DeploymentLifecycle::from_str("recovery_required").expect("state"),
        DeploymentLifecycle::RecoveryRequired
    );
    assert!(matches!(
        DeploymentLifecycle::from_str("provider_custom"),
        Err(DeploymentError::UnknownState)
    ));
    assert!(InstallProgress::from_str("future").is_err());
    assert!(RemovalProgress::from_str("future").is_err());
    assert!(DiagnosticCode::from_str("raw provider message").is_err());
}
