//! Controlled decoded-ledger comparisons only; SQL provenance remains a real PG gate.
use super::{Completion, validate};
use crate::{effect_state::State, execution_transitions};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, InstallProgress, RemovalProgress,
    ResourceAction,
};
use serde_json::json;
use std::collections::BTreeMap;
use uuid::Uuid;

fn fixture() -> (Completion, EffectClaim, DeploymentSnapshot, State) {
    let (intent, mut claim, _) = crate::remove_resume::decoding::tests::fixture();
    claim.action = ResourceAction::Delete;
    claim.resource_version = 7;
    claim.generation = 4;
    let before = State {
        install: InstallProgress::Ready,
        removal: RemovalProgress::Detached,
        version: 7,
        active_attempt: Some(claim.provenance.attempt_id.as_uuid()),
        diagnostic: None,
        generation: 4,
        last_action: Some(ResourceAction::Detach),
    };
    let mut current = before.applied(ResourceAction::Delete);
    current.version += 1;
    let snapshot = DeploymentSnapshot {
        intent,
        lifecycle: DeploymentLifecycle::Removing,
        version: 20,
        resources: BTreeMap::from([(claim.resource.clone(), current.progress().unwrap())]),
    };
    let operation = claim.provenance.attempt_id.as_uuid();
    let row = Completion {
        id: execution_transitions::transition_id(operation, "completed"),
        command_id: claim.command.id().as_uuid(),
        operation_id: operation,
        kind: "completed".into(),
        fingerprint: execution_transitions::fingerprint(
            &claim,
            "completed",
            &json!(["applied", null]),
        )
        .unwrap()
        .as_bytes()
        .to_vec(),
        actor_id: claim.command.actor_id().as_uuid(),
        request_id: claim.provenance.request_id.as_uuid(),
        command_actor: claim.command.actor_id().as_uuid(),
        idempotency_id: claim.command.idempotency_id().as_uuid(),
        command_operation: "remove".into(),
        before_state: sqlx::types::Json(json!(before)),
        after_state: sqlx::types::Json(json!(current)),
        before_deployment_version: 10,
        after_deployment_version: 11,
        before_lifecycle: "removing".into(),
        after_lifecycle: "removing".into(),
        verification_id: Some(Uuid::from_u128(100)),
        evidence_valid: true,
    };
    (row, claim, snapshot, current)
}

#[test]
fn original_completion_requires_exact_original_request_and_before_version() {
    let (row, claim, snapshot, current) = fixture();
    validate(&row, &claim, &snapshot, &current).unwrap();
    let mut changed = row.clone();
    changed.request_id = Uuid::from_u128(101);
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = row;
    changed.before_state.0["version"] = json!(8);
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
}

#[test]
fn current_reconciliation_is_distinct_from_saved_original_publisher() {
    let (mut row, claim, snapshot, current) = fixture();
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(110)),
        RequestId::from_uuid(Uuid::from_u128(111)),
        DeploymentOperation::Remove,
    )
    .unwrap();
    let operation = DeploymentAttemptId::from_uuid(Uuid::from_u128(112)).unwrap();
    let provenance = AttemptProvenance {
        attempt_id: operation,
        actor_id: command.actor_id(),
        request_id: RequestId::from_uuid(Uuid::from_u128(113)),
    };
    row.command_id = command.id().as_uuid();
    row.command_actor = command.actor_id().as_uuid();
    row.actor_id = command.actor_id().as_uuid();
    row.idempotency_id = command.idempotency_id().as_uuid();
    row.request_id = provenance.request_id.as_uuid();
    row.operation_id = operation.as_uuid();
    row.kind = "reconciled".into();
    row.id = execution_transitions::transition_id(operation.as_uuid(), "reconciled");
    row.fingerprint = execution_transitions::fingerprint(
        &claim,
        "reconciled",
        &json!([command, provenance, "applied", null]),
    )
    .unwrap()
    .as_bytes()
    .to_vec();
    validate(&row, &claim, &snapshot, &current).unwrap();
    assert_ne!(row.actor_id, claim.provenance.actor_id.as_uuid());
    assert_ne!(row.request_id, claim.provenance.request_id.as_uuid());
    row.request_id = claim.provenance.request_id.as_uuid();
    assert!(validate(&row, &claim, &snapshot, &current).is_err());
}

#[test]
fn substituted_event_proof_fingerprint_state_generation_and_progress_deny() {
    let (row, claim, snapshot, current) = fixture();
    let mut changed = row.clone();
    changed.evidence_valid = false;
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = row.clone();
    changed.verification_id = None;
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = row.clone();
    changed.fingerprint[0] ^= 1;
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = row.clone();
    changed.before_state.0["generation"] = json!(5);
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = row.clone();
    changed.before_state.0["last_action"] = json!("drain");
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = row.clone();
    changed.after_deployment_version += 1;
    assert!(validate(&changed, &claim, &snapshot, &current).is_err());
    let mut changed = current;
    changed.active_attempt = Some(claim.provenance.attempt_id.as_uuid());
    assert!(validate(&row, &claim, &snapshot, &changed).is_err());
}
