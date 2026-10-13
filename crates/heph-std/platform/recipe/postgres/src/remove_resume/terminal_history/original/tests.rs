//! Controlled wire selection, never a substitute for native/ledger qualification.
use super::selected;
use crate::{execution_rows::ClaimWire, remove_resume::decoding};
use capability_domain::CapabilitySlotKey;
use identity_domain::{RequestId, UserId};
use recipe_application::{CommandIdentity, DeploymentOperation, ResourceAction};
use serde_json::{Value, json};
use uuid::Uuid;

fn original() -> (Value, CommandIdentity, CapabilitySlotKey) {
    let (_, mut claim, _) = decoding::tests::fixture();
    claim.action = ResourceAction::Delete;
    claim.resource_version = 7;
    claim.generation = 4;
    let raw = json!({"resource":claim.resource,"delete_claim":ClaimWire::new(&claim).unwrap(),
        "observed_progress":{"install":"ready","removal":"deleted","version":8,
            "active_attempt":null,"diagnostic":null,"generation":4,"last_action":"delete"},
        "before_instance_version":11,"terminal_operation_id":Uuid::from_u128(900),
        "after_instance_version":12,"publisher":{"command_id":claim.command.id(),
            "actor_id":claim.command.actor_id(),"request_id":claim.provenance.request_id,"admission_command_id":null}});
    (json!({"terminal":[raw]}), claim.command, claim.resource)
}
#[test]
fn original_only_selection_rejects_distinct_manager_and_foreign_resource() {
    let (data, command, resource) = original();
    assert_eq!(
        selected(&data, command, &resource).unwrap(),
        &data["terminal"][0]
    );
    let fresh = CommandIdentity::from_recorded(
        UserId::from_uuid(Uuid::from_u128(600)),
        RequestId::from_uuid(Uuid::from_u128(601)),
        DeploymentOperation::Remove,
    )
    .unwrap();
    assert!(selected(&data, fresh, &resource).is_err());
    assert!(
        selected(
            &data,
            command,
            &CapabilitySlotKey::parse("foreign").unwrap()
        )
        .is_err()
    );
    let mut wrong = data;
    wrong["terminal"][0]["delete_claim"]["command_id"] = json!(fresh.id());
    assert!(selected(&wrong, command, &resource).is_err());
}
#[test]
fn original_replay_denies_missing_active_and_unproven_terminal_shapes() {
    let (data, command, resource) = original();
    for (field, value) in [
        ("terminal_operation_id", Value::Null),
        ("publisher", Value::Null),
        ("before_instance_version", json!(0)),
        ("unknown", json!(true)),
    ] {
        let mut wrong = data.clone();
        wrong["terminal"][0][field] = value;
        assert!(selected(&wrong, command, &resource).is_err(), "{field}");
    }
    let mut active = data.clone();
    active["terminal"][0]["observed_progress"] = json!({"install":"ready","removal":"detached", "version":7,
        "active_attempt":data["terminal"][0]["delete_claim"]["attempt_id"],"diagnostic":null,"generation":4,"last_action":"detach"});
    assert!(selected(&active, command, &resource).is_err());
    assert!(selected(&json!({"terminal":[]}), command, &resource).is_err());
    assert!(selected(&json!({}), command, &resource).is_err());
}
#[test]
fn original_replay_denies_duplicate_history_and_changed_generation_or_cas() {
    let (data, command, resource) = original();
    let mut duplicated = data.clone();
    duplicated["terminal"]
        .as_array_mut()
        .unwrap()
        .push(data["terminal"][0].clone());
    assert!(selected(&duplicated, command, &resource).is_err());
    for (field, value) in [
        ("generation", json!(5)),
        ("last_action", json!("detach")),
        ("diagnostic", json!("drain_pending")),
    ] {
        let mut wrong = data.clone();
        wrong["terminal"][0]["observed_progress"][field] = value;
        assert!(selected(&wrong, command, &resource).is_err(), "{field}");
    }
    let mut wrong = data;
    wrong["terminal"][0]["after_instance_version"] = json!(13);
    assert!(selected(&wrong, command, &resource).is_err());
}
