//! Controlled immutable-row validation; no worker receipt or physical proof.
use super::{Completion, decode_state, validate};
use crate::effect_state::State;
use crate::execution_transitions;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, DiagnosticCode, EffectClaim, InstallProgress,
    RemovalProgress, ResourceAction,
};
use serde_json::json;
use std::collections::BTreeMap;
use uuid::Uuid;

fn fixture() -> (Completion, EffectClaim, DeploymentSnapshot, State) {
    let (intent, claim, _) = super::super::decoding::tests::fixture();
    let snapshot = DeploymentSnapshot {
        intent,
        lifecycle: DeploymentLifecycle::Removing,
        version: 9,
        resources: BTreeMap::new(),
    };
    let before = State {
        install: InstallProgress::Ready,
        removal: RemovalProgress::Draining,
        version: 3,
        active_attempt: Some(claim.provenance.attempt_id.as_uuid()),
        diagnostic: None,
        generation: 2,
        last_action: Some(ResourceAction::Create),
    };
    let mut after = before.applied(ResourceAction::Drain);
    after.version = 4;
    let completed = Completion {
        id: execution_transitions::transition_id(
            claim.provenance.attempt_id.as_uuid(),
            "completed",
        ),
        command_id: claim.command.id().as_uuid(),
        operation_id: claim.provenance.attempt_id.as_uuid(),
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
        after_state: sqlx::types::Json(json!(after)),
        before_deployment_version: 5,
        after_deployment_version: 6,
        before_lifecycle: "removing".into(),
        after_lifecycle: "removing".into(),
        verification_id: Some(Uuid::from_u128(20)),
        evidence_valid: true,
    };
    (completed, claim, snapshot, after)
}

#[test]
fn later_generation_active_or_detached_state_cannot_replay_old_completed_drain() {
    let (row, claim, snapshot, state) = fixture();
    validate(&row, &claim, &snapshot, &state).unwrap();
    for mutation in [0, 1, 2, 3] {
        let mut changed = state.clone();
        match mutation {
            0 => changed.generation += 1,
            1 => changed.active_attempt = Some(Uuid::from_u128(99)),
            2 => {
                changed.removal = RemovalProgress::Detached;
                changed.last_action = Some(ResourceAction::Detach);
            }
            _ => changed.diagnostic = Some(DiagnosticCode::DrainPending),
        }
        assert!(validate(&row, &claim, &snapshot, &changed).is_err());
    }
}

#[test]
fn missing_receipt_event_ancestry_changed_command_and_fingerprint_deny_completion() {
    let (row, claim, snapshot, state) = fixture();
    for mutation in 0..9 {
        let mut changed = row.clone();
        match mutation {
            0 => changed.evidence_valid = false,
            1 => changed.actor_id = Uuid::from_u128(99),
            2 => changed.command_id = Uuid::from_u128(99),
            3 => changed.operation_id = Uuid::from_u128(99),
            4 => changed.fingerprint[0] ^= 1,
            5 => changed.after_deployment_version = 10,
            6 => changed.after_lifecycle = "installed".into(),
            7 => changed.verification_id = None,
            _ => changed.before_state.0["extra"] = json!(true),
        }
        assert!(
            validate(&changed, &claim, &snapshot, &state).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn direct_completion_cannot_substitute_a_different_non_nil_original_request() {
    let (mut row, claim, snapshot, state) = fixture();
    validate(&row, &claim, &snapshot, &state).unwrap();
    row.request_id = Uuid::from_u128(99);
    assert!(!row.request_id.is_nil());
    assert_ne!(row.request_id, claim.provenance.request_id.as_uuid());
    // Even a coherently matching event/receipt witness cannot replace provenance.
    assert!(row.evidence_valid);
    assert!(validate(&row, &claim, &snapshot, &state).is_err());
}

#[test]
fn reconciliation_uses_original_claim_but_real_distinct_current_manager_provenance() {
    let (mut row, claim, snapshot, _) = fixture();
    let current = CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(30)),
        RequestId::from_uuid(Uuid::from_u128(31)),
        DeploymentOperation::Remove,
    )
    .unwrap();
    let provenance = AttemptProvenance {
        attempt_id: DeploymentAttemptId::from_uuid(Uuid::from_u128(32)).unwrap(),
        actor_id: current.actor_id(),
        request_id: RequestId::from_uuid(Uuid::from_u128(33)),
    };
    row.command_id = current.id().as_uuid();
    row.actor_id = current.actor_id().as_uuid();
    row.command_actor = row.actor_id;
    row.idempotency_id = current.idempotency_id().as_uuid();
    row.request_id = provenance.request_id.as_uuid();
    row.operation_id = provenance.attempt_id.as_uuid();
    row.kind = "reconciled".into();
    row.id = execution_transitions::transition_id(row.operation_id, "reconciled");
    row.fingerprint = execution_transitions::fingerprint(
        &claim,
        "reconciled",
        &json!([current, provenance, "applied", null]),
    )
    .unwrap()
    .as_bytes()
    .to_vec();
    let mut before = decode_state(&row.before_state.0).unwrap();
    before.version = 4;
    before.removal = RemovalProgress::RecoveryRequired;
    before.diagnostic = Some(DiagnosticCode::ProviderOutcomeUnknown);
    let mut state = before.applied(ResourceAction::Drain);
    state.version = 5;
    row.before_state.0 = json!(before);
    row.after_state.0 = json!(state);
    row.before_lifecycle = "recovery_required".into();
    validate(&row, &claim, &snapshot, &state).unwrap();
    assert_ne!(current.actor_id(), claim.command.actor_id());
    row.request_id = Uuid::nil();
    assert!(validate(&row, &claim, &snapshot, &state).is_err());
}
