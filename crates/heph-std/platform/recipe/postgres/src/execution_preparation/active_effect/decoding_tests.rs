//! Pure corrupt-row decoding tests; labels here convey no physical or SQL proof.
use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentId,
    DeploymentIntent, DeploymentKey, DeploymentLifecycle, DeploymentOperation, DeploymentSnapshot,
    EffectClaim, InstallProgress, RemovalProgress, ResourceAction,
};
use release_domain::ContentHash;
use uuid::Uuid;

use super::{ActiveRow, ClaimWire, State, restore};

fn intent() -> DeploymentIntent {
    let recipe = recipe_domain::parse_recipe(
        br#"
contract_version=1
recipe_id="decode"
recipe_version="1.0.0"
[[resources]]
kind="volume"
name="data"
source={type="created",capacity_bytes={source="literal",value=16777216}}
"#,
    )
    .unwrap();
    let resolved = recipe
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[])
        .unwrap();
    DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::from_u128(1)).unwrap(),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("decode").unwrap(),
        &recipe,
        &resolved,
    )
    .unwrap()
}

fn fixture() -> (DeploymentSnapshot, ActiveRow, EffectClaim) {
    let intent = intent();
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(3)),
        RequestId::from_uuid(Uuid::from_u128(4)),
        DeploymentOperation::Install,
    )
    .unwrap();
    let claim = EffectClaim {
        command,
        deployment_id: intent.id(),
        resource: CapabilitySlotKey::parse("data").unwrap(),
        identity: intent.resources().values().next().unwrap().identity(),
        input_hash: intent.resources().values().next().unwrap().input_hash(),
        resource_version: 1,
        generation: 1,
        action: ResourceAction::Create,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(Uuid::from_u128(5)).unwrap(),
            actor_id: command.actor_id(),
            request_id: RequestId::from_uuid(Uuid::from_u128(6)),
        },
    };
    let before = State {
        install: InstallProgress::Pending,
        removal: RemovalProgress::Pending,
        version: 0,
        active_attempt: None,
        diagnostic: None,
        generation: 0,
        last_action: None,
    };
    let request = serde_json::to_vec(&(
        command,
        intent.id(),
        &claim.resource,
        0_i64,
        0_i64,
        claim.action,
        claim.provenance,
    ))
    .unwrap();
    let row = ActiveRow {
        id: claim.provenance.attempt_id.as_uuid(),
        command_id: command.id().as_uuid(),
        deployment_id: intent.id().as_uuid(),
        project_id: intent.project_id().as_uuid(),
        resource_name: "data".into(),
        action: "create".into(),
        generation: 1,
        input_hash: claim.input_hash.as_bytes().to_vec(),
        claim_json: sqlx::types::Json(
            serde_json::to_value(ClaimWire::new(&claim).unwrap()).unwrap(),
        ),
        before_state: sqlx::types::Json(serde_json::to_value(&before).unwrap()),
        request_fingerprint: ContentHash::digest(&request).as_bytes().to_vec(),
        expected_deployment_version: 0,
        actor_id: command.actor_id().as_uuid(),
        request_id: claim.provenance.request_id.as_uuid(),
        command_actor: command.actor_id().as_uuid(),
        idempotency_id: command.idempotency_id().as_uuid(),
        operation: "install".into(),
        claimed_state: Some(sqlx::types::Json(
            serde_json::to_value(before.claim(&claim).unwrap()).unwrap(),
        )),
        proof_valid: true,
    };
    (
        DeploymentSnapshot {
            intent,
            lifecycle: DeploymentLifecycle::Installing,
            version: 1,
            resources: BTreeMap::new(),
        },
        row,
        claim,
    )
}

#[test]
fn every_wire_field_is_compared_to_checked_command_plan_and_original_attempt() {
    let (snapshot, row, claim) = fixture();
    let plan = &snapshot.intent.resources()[&claim.resource];
    assert_eq!(
        restore(&row, &snapshot, plan, &claim.resource).unwrap(),
        claim
    );
    for field in [
        "command_id",
        "deployment_id",
        "resource",
        "identity",
        "input_hash",
        "resource_version",
        "generation",
        "action",
        "attempt_id",
        "actor_id",
        "request_id",
    ] {
        let (_, mut changed, _) = fixture();
        changed.claim_json.0[field] = match field {
            "resource" => serde_json::json!("foreign"),
            "identity" => serde_json::json!({"Volume":{"id":Uuid::from_u128(99)}}),
            "input_hash" => serde_json::json!(vec![0_u8; 32]),
            "resource_version" | "generation" => serde_json::json!(2),
            "action" => serde_json::json!("detach"),
            _ => serde_json::json!(Uuid::from_u128(99)),
        };
        assert!(
            matches!(
                restore(&changed, &snapshot, plan, &claim.resource),
                Err(DeploymentError::ReconciliationRequired)
            ),
            "{field}"
        );
    }
}

#[test]
fn relational_generation_hash_provenance_and_claimed_proof_are_not_wire_only() {
    let (snapshot, mut row, claim) = fixture();
    let plan = &snapshot.intent.resources()[&claim.resource];
    row.generation = 2;
    assert!(matches!(
        restore(&row, &snapshot, plan, &claim.resource),
        Err(DeploymentError::ReconciliationRequired)
    ));
    let (_, mut row, _) = fixture();
    row.input_hash = vec![0; 32];
    assert!(matches!(
        restore(&row, &snapshot, plan, &claim.resource),
        Err(DeploymentError::ReconciliationRequired)
    ));
    let (_, mut row, _) = fixture();
    row.request_id = Uuid::new_v4();
    assert!(matches!(
        restore(&row, &snapshot, plan, &claim.resource),
        Err(DeploymentError::ReconciliationRequired)
    ));
    let (_, mut row, _) = fixture();
    row.proof_valid = false;
    assert!(matches!(
        restore(&row, &snapshot, plan, &claim.resource),
        Err(DeploymentError::ReconciliationRequired)
    ));
    let (_, mut row, _) = fixture();
    row.claimed_state = None;
    assert!(matches!(
        restore(&row, &snapshot, plan, &claim.resource),
        Err(DeploymentError::ReconciliationRequired)
    ));
}

#[test]
fn later_progress_does_not_rewrite_original_claim_version_or_request() {
    let (mut snapshot, row, claim) = fixture();
    snapshot.version = 7;
    assert_eq!(
        restore(
            &row,
            &snapshot,
            &snapshot.intent.resources()[&claim.resource],
            &claim.resource
        )
        .unwrap(),
        claim
    );
}
