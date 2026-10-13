//! Controlled comparison rows; these tests produce no native or physical receipt.
use super::{ledger, retained};
use crate::{effect_state::State, execution_transitions};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentId, DeploymentOperation,
    EffectClaim, InstallProgress, PlannedResourceIdentity, RemovalProgress, ResourceAction,
};
use release_domain::ContentHash;
use serde_json::json;
use uuid::Uuid;

fn claim() -> EffectClaim {
    let actor = UserId::new();
    EffectClaim {
        command: CommandIdentity::from_recorded(
            actor,
            RequestId::new(),
            DeploymentOperation::Install,
        )
        .unwrap(),
        deployment_id: DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        resource: capability_domain::CapabilitySlotKey::parse("data").unwrap(),
        identity: PlannedResourceIdentity::Volume {
            id: runtime_types::VolumeId::from_uuid(Uuid::new_v4()),
            filesystem_uuid: Some(Uuid::new_v4()),
        },
        input_hash: ContentHash::digest(b"whole-volume-plan"),
        resource_version: 1,
        generation: 1,
        action: ResourceAction::Create,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
            actor_id: actor,
            request_id: RequestId::new(),
        },
    }
}
fn completion(claim: &EffectClaim) -> ledger::Completion {
    let before = State {
        install: InstallProgress::Creating,
        removal: RemovalProgress::Pending,
        version: 1,
        active_attempt: Some(claim.provenance.attempt_id.as_uuid()),
        diagnostic: None,
        generation: 1,
        last_action: None,
    };
    let mut after = before.applied(ResourceAction::Create);
    after.version = 2;
    ledger::Completion {
        id: execution_transitions::transition_id(
            claim.provenance.attempt_id.as_uuid(),
            "completed",
        ),
        command_id: claim.command.id().as_uuid(),
        operation_id: claim.provenance.attempt_id.as_uuid(),
        kind: "completed".into(),
        fingerprint: execution_transitions::fingerprint(
            claim,
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
        command_operation: "install".into(),
        before_state: sqlx::types::Json(serde_json::to_value(before).unwrap()),
        after_state: sqlx::types::Json(serde_json::to_value(after).unwrap()),
        before_deployment_version: 1,
        after_deployment_version: 2,
        before_lifecycle: "installing".into(),
        after_lifecycle: "installing".into(),
        verification_id: Some(Uuid::new_v4()),
        evidence_valid: true,
    }
}

#[test]
fn recorded_create_requires_original_request_and_positive_applied_evidence() {
    let claim = claim();
    let mut row = completion(&claim);
    assert!(ledger::validate(&row, &claim, 4).is_ok());
    row.request_id = Uuid::new_v4();
    assert!(ledger::validate(&row, &claim, 4).is_err());
    row.request_id = claim.provenance.request_id.as_uuid();
    row.evidence_valid = false;
    assert!(ledger::validate(&row, &claim, 4).is_err());
    row.evidence_valid = true;
    row.verification_id = None;
    assert!(ledger::validate(&row, &claim, 4).is_err());
}

#[test]
fn foreign_generation_or_unbounded_state_cannot_supply_original_ready() {
    let claim = claim();
    let mut row = completion(&claim);
    row.before_state.0["generation"] = json!(2);
    row.after_state.0["generation"] = json!(2);
    assert!(ledger::validate(&row, &claim, 4).is_err());
    let mut row = completion(&claim);
    row.after_state.0["invented_proof"] = json!(true);
    assert!(ledger::validate(&row, &claim, 4).is_err());
    let mut row = completion(&claim);
    row.after_deployment_version = 5;
    assert!(ledger::validate(&row, &claim, 4).is_err());
}

#[test]
fn original_mapping_requires_one_exact_canonical_row() {
    let expected = b"actual-full-claim";
    assert!(super::super::birth::canonical(&[expected.to_vec()], expected).is_ok());
    assert!(super::super::birth::canonical(&[], expected).is_err());
    assert!(
        super::super::birth::canonical(&[expected.to_vec(), expected.to_vec()], expected).is_err()
    );
    assert!(super::super::birth::canonical(&[b"foreign-full-claim".to_vec()], expected).is_err());
}

fn retention(claim: &EffectClaim, ready: &State) -> (retained::Retention, State) {
    let command = CommandIdentity::from_recorded(
        UserId::new(),
        RequestId::new(),
        DeploymentOperation::Remove,
    )
    .unwrap();
    let operation = DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap();
    let mut after = ready.clone();
    after.version += 1;
    after.removal = RemovalProgress::Retained;
    after.last_action = Some(ResourceAction::Retain);
    let row = retained::Retention {
        id: execution_transitions::transition_id(operation.as_uuid(), "creation_retained_ready"),
        command_id: command.id().as_uuid(),
        operation_id: operation.as_uuid(),
        fingerprint: execution_transitions::fingerprint(
            claim,
            "creation_retained_ready",
            &json!([command, operation, 4, ready.version]),
        )
        .unwrap()
        .as_bytes()
        .to_vec(),
        actor_id: command.actor_id().as_uuid(),
        request_id: Uuid::new_v4(),
        command_actor: command.actor_id().as_uuid(),
        idempotency_id: command.idempotency_id().as_uuid(),
        before_state: sqlx::types::Json(serde_json::to_value(ready).unwrap()),
        after_state: sqlx::types::Json(serde_json::to_value(&after).unwrap()),
        before_deployment_version: 4,
        after_deployment_version: 5,
        before_lifecycle: "removing".into(),
        after_lifecycle: "removing".into(),
        evidence_valid: true,
    };
    (row, after)
}

#[test]
fn retained_disposition_requires_actual_fact_and_same_original_generation() {
    let claim = claim();
    let ready = ledger::validate(&completion(&claim), &claim, 2).unwrap();
    let (mut row, after) = retention(&claim, &ready);
    assert!(retained::validate(&row, &claim, &ready, &after, 5).is_ok());
    row.evidence_valid = false;
    assert!(retained::validate(&row, &claim, &ready, &after, 5).is_err());
    row.evidence_valid = true;
    let mut foreign = after.clone();
    foreign.generation += 1;
    assert!(retained::validate(&row, &claim, &ready, &foreign, 5).is_err());
    row.operation_id = claim.provenance.attempt_id.as_uuid();
    assert!(retained::validate(&row, &claim, &ready, &after, 5).is_err());
}
