//! Controlled corrupt-row tests, not database or cleanup evidence.
use super::{ClosureRow, decode};
use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentId, DeploymentIntent,
    DeploymentKey, DeploymentOperation, EffectClaim, OriginalInstanceClosure, ResourceAction,
};
use recipe_domain::{ReleaseCatalogEntry, ReleasePin};
use release_domain::{InstanceRemovalId, ReleaseAgentId, ReleaseCommandKey, ReleaseId};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

pub fn fixture() -> (DeploymentIntent, EffectClaim, OriginalInstanceClosure) {
    let pin = ReleasePin {
        release_id: ReleaseId::from_uuid(Uuid::from_u128(10)),
        release_agent_id: ReleaseAgentId::from_uuid(Uuid::from_u128(11)),
    };
    let source = format!(
        "contract_version=1\nrecipe_id=\"decode\"\nrecipe_version=\"1.0.0\"\n[[resources]]\nkind=\"instance\"\nname=\"worker\"\nrelease={{release_id=\"{}\",release_agent_id=\"{}\"}}\n",
        pin.release_id, pin.release_agent_id
    );
    let declaration = recipe_domain::parse_recipe(source.as_bytes()).unwrap();
    let catalog = ReleaseCatalogEntry {
        pin,
        published: true,
        parameters: vec![],
        volume_slots: vec![],
        required_secret_slot_count: 0,
        capability_requirements: vec![],
    };
    let resolved = declaration
        .resolve(&BTreeMap::new(), &BTreeMap::new(), &[catalog])
        .unwrap();
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::from_u128(1)).unwrap(),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("decode").unwrap(),
        &declaration,
        &resolved,
    )
    .unwrap();
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(3)),
        RequestId::from_uuid(Uuid::from_u128(4)),
        DeploymentOperation::Remove,
    )
    .unwrap();
    let resource = CapabilitySlotKey::parse("worker").unwrap();
    let plan = &intent.resources()[&resource];
    let claim = EffectClaim {
        command,
        deployment_id: intent.id(),
        resource,
        identity: plan.identity(),
        input_hash: plan.input_hash(),
        resource_version: 3,
        generation: 2,
        action: ResourceAction::Drain,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(Uuid::from_u128(5)).unwrap(),
            actor_id: command.actor_id(),
            request_id: RequestId::from_uuid(Uuid::from_u128(6)),
        },
    };
    let recipe_application::PlannedResourceIdentity::Instance { id, .. } = claim.identity else {
        panic!("instance fixture")
    };
    let mut bytes = b"hephaestus-prepared-instance-removal-v1\0".to_vec();
    bytes.extend_from_slice(id.as_uuid().as_bytes());
    let closure = OriginalInstanceClosure {
        publication_origin: recipe_application::InstanceClosurePublicationOrigin::OriginalDrain,
        drain: claim.clone(),
        removal_id: InstanceRemovalId::from_uuid(Uuid::new_v5(
            &claim.provenance.attempt_id.as_uuid(),
            &bytes,
        )),
        command_key: ReleaseCommandKey::derive(
            "prepared_instance_remove_v1",
            &[
                id.as_uuid().as_bytes(),
                claim.provenance.attempt_id.as_uuid().as_bytes(),
            ],
        ),
        expected_version: 2,
        actor: command.actor_id(),
        request: claim.provenance.request_id,
        input_hash: release_domain::ContentHash::digest(b"controlled closure fingerprint"),
    };
    (intent, claim, closure)
}

pub(super) fn row(claim: &EffectClaim, closure: &OriginalInstanceClosure) -> ClosureRow {
    ClosureRow {
        resource: claim.resource.as_str().into(),
        source: json!({"drain_claim": crate::execution_rows::ClaimWire::new(claim).unwrap(),
            "actor": claim.command.actor_id(), "idempotency": claim.command.idempotency_id(),
            "resource": claim.resource, "removal": {}, "inbox": {}}),
        id: closure.removal_id.as_uuid(),
        key: serde_json::from_value(serde_json::to_value(closure.command_key).unwrap()).unwrap(),
        expected: 2,
        actor: closure.actor.as_uuid(),
        request: closure.request.as_uuid(),
        hash: closure.input_hash.as_bytes().to_vec(),
    }
}

fn data(claims: &Value, rows: &[ClosureRow]) -> Value {
    json!({"claims": claims, "closures": rows.iter().map(|row| &row.source).collect::<Vec<_>>()})
}

#[test]
fn complete_native_closure_list_survives_an_empty_active_claim_list() {
    let (intent, claim, closure) = fixture();
    let rows = [row(&claim, &closure)];
    let decoded = decode(&data(&json!([]), &rows), &rows, &intent, None).unwrap();
    assert!(decoded.continuations.is_empty());
    assert_eq!(decoded.closures, vec![closure]);
}

#[test]
fn duplicate_resource_foreign_claim_and_unknown_wire_fields_deny_whole_history() {
    let (intent, claim, closure) = fixture();
    let duplicate = [row(&claim, &closure), row(&claim, &closure)];
    assert!(decode(&data(&json!([]), &duplicate), &duplicate, &intent, None).is_err());
    let mut wrong = row(&claim, &closure);
    wrong.resource = "foreign".into();
    assert!(
        decode(
            &data(&json!([]), std::slice::from_ref(&wrong)),
            &[wrong],
            &intent,
            None,
        )
        .is_err()
    );
    for field in ["deployment_id", "input_hash", "extra"] {
        let mut wrong = row(&claim, &closure);
        wrong.source["drain_claim"][field] = json!(Uuid::from_u128(99));
        assert!(
            decode(
                &data(&json!([]), std::slice::from_ref(&wrong)),
                &[wrong],
                &intent,
                None,
            )
            .is_err(),
            "{field}"
        );
    }
}

#[test]
fn malformed_closure_identity_and_active_claim_duplicates_do_not_return_partial_results() {
    let (intent, claim, closure) = fixture();
    for mutation in [0, 1, 2] {
        let mut wrong = row(&claim, &closure);
        match mutation {
            0 => wrong.id = Uuid::from_u128(99),
            1 => wrong.key[0] ^= 1,
            _ => wrong.expected = 0,
        }
        assert!(
            decode(
                &data(&json!([]), std::slice::from_ref(&wrong)),
                &[wrong],
                &intent,
                None,
            )
            .is_err()
        );
    }
    let value = json!({"claim": crate::execution_rows::ClaimWire::new(&claim).unwrap(),
        "actor": claim.command.actor_id(), "idempotency": claim.command.idempotency_id()});
    let rows = [row(&claim, &closure)];
    assert!(
        decode(
            &data(&json!([value.clone(), value]), &rows),
            &rows,
            &intent,
            None
        )
        .is_err()
    );
    assert!(decode(&json!({"claims": []}), &[], &intent, None).is_err());
}
