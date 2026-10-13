//! Controlled wire substitutions only; no PG originals or native proof are fabricated.
use super::super::{decode, inventory, terminal, wire};
use super::fixture::Fixture;
use crate::execution_rows::ClaimWire;
use recipe_application::ResourceAction;
use serde_json::{Value, json};
use uuid::Uuid;

fn terminal() -> Value {
    let fixture = Fixture::new();
    let mut claim = fixture.drain;
    claim.action = ResourceAction::Delete;
    claim.generation = 4;
    claim.resource_version = 7;
    json!({"resource":claim.resource,"delete_claim":ClaimWire::new(&claim).unwrap(),
        "observed_progress":{"install":"ready","removal":"detached","version":7,
            "active_attempt":claim.provenance.attempt_id,"diagnostic":null,"generation":4,"last_action":"detach"},
        "before_instance_version":11,"terminal_operation_id":null,"after_instance_version":null,"publisher":null})
}
fn committed() -> Value {
    let mut value = terminal();
    value["terminal_operation_id"] = json!(Uuid::from_u128(900));
    value["after_instance_version"] = json!(12);
    value["publisher"] = json!({"command_id":Uuid::from_u128(901),"actor_id":Uuid::from_u128(902),
        "request_id":Uuid::from_u128(903),"admission_command_id":Uuid::from_u128(901)});
    value
}

#[test]
fn terminal_preserves_full_claim_state_and_original_before_cas() {
    let value = committed();
    let rows = terminal::history_entries(&json!({"terminal":[value]})).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(
        serde_json::to_value(&row.claim).unwrap(),
        value["delete_claim"]
    );
    assert_eq!(
        serde_json::to_value(&row.state).unwrap(),
        value["observed_progress"]
    );
    assert_eq!(row.before, Some(11));
    assert_eq!(row.after, Some(12));
    assert_eq!(row.publisher.as_ref(), Some(&value["publisher"]));
}

#[test]
fn terminal_denies_every_partial_committed_tuple_and_inactive_unwritten() {
    let value = committed();
    for mask in 1_u8..7 {
        let mut changed = value.clone();
        for (index, name) in [
            "terminal_operation_id",
            "after_instance_version",
            "publisher",
        ]
        .iter()
        .enumerate()
        {
            if mask & (1 << index) == 0 {
                changed[*name] = Value::Null;
            }
        }
        assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
    }
    let mut changed = terminal();
    changed["observed_progress"]["active_attempt"] = Value::Null;
    assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
}

#[test]
fn terminal_denies_extra_fields_bad_publisher_and_cas_substitutions() {
    let value = committed();
    for path in ["before_instance_version", "after_instance_version"] {
        for replacement in [json!(0), json!(-1), json!(i64::MAX), json!(13)] {
            let mut changed = value.clone();
            changed[path] = replacement;
            assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
        }
    }
    for name in [
        "command_id",
        "actor_id",
        "request_id",
        "admission_command_id",
    ] {
        let mut changed = value.clone();
        changed["publisher"][name] = json!(Uuid::nil());
        assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
    }
    for area in ["publisher", "observed_progress", "delete_claim"] {
        let mut changed = value.clone();
        changed[area]["unknown"] = json!(1);
        assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
    }
    for name in [
        "attempt_id",
        "command_id",
        "deployment_id",
        "actor_id",
        "request_id",
    ] {
        let mut changed = value.clone();
        changed["delete_claim"][name] = json!(Uuid::nil());
        assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
    }
    let mut changed = value.clone();
    changed["terminal_operation_id"] = json!(Uuid::nil());
    assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
    let mut changed = value;
    changed["unknown"] = json!(1);
    assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
}

#[test]
fn optional143_inventories_are_nonempty_bounded_unique_and_disjoint() {
    for name in ["detach", "terminal"] {
        for replacement in [json!(null), json!([]), json!({})] {
            let mut data = json!({"claims":[],"closures":[]});
            data[name] = replacement;
            assert!(wire::context(&data, 1).is_err());
        }
    }
    let value = terminal();
    assert!(terminal::Native143::parse(&json!({"terminal":[value,value]}), 2).is_err());
    assert!(terminal::Native143::parse(&json!({"terminal":[value]}), 0).is_err());
    let mut detach = value["delete_claim"].clone();
    detach["action"] = json!("detach");
    detach["generation"] = json!(3);
    detach["resource_version"] = json!(5);
    let detach = json!({"resource":"worker","detach_claim":detach,
        "observed_progress":{"install":"ready","removal":"recovery_required","version":6,
            "active_attempt":value["delete_claim"]["attempt_id"],"diagnostic":"drain_pending","generation":3,"last_action":"drain"}});
    assert!(terminal::Native143::parse(&json!({"detach":[detach]}), 1).is_ok());
    let data = json!({"detach":[detach],"terminal":[value]});
    assert!(terminal::Native143::parse(&data, 2).is_err());
    for name in ["pending", "recovery"] {
        let mut data = json!({"terminal":[value]});
        data[name] = json!([{"resource":"worker"}]);
        assert!(terminal::Native143::parse(&data, 2).is_err());
    }
}

#[test]
fn raw_terminal_wire_cannot_authorize_current_inventory_without_loaded_originals() {
    let fixture = Fixture::new();
    let value = terminal();
    // Retain the existing strict OriginalDrain test row rather than pretending
    // the transport payload has loaded immutable DB producer rows.
    let (_, _, closure) = super::super::tests::fixture();
    let row = super::super::tests::row(&fixture.drain, &closure);
    let data = json!({"claims":[{"claim":value["delete_claim"],
        "actor":fixture.drain.command.actor_id(),"idempotency":fixture.drain.command.idempotency_id()}],
        "closures":[row.source],"terminal":[value]});
    let mut payload_only = data.clone();
    payload_only.as_object_mut().unwrap().remove("terminal");
    assert!(
        decode(
            &payload_only,
            std::slice::from_ref(&row),
            &fixture.intent,
            Some(&fixture.roots())
        )
        .is_err()
    );
    let decoded = decode(&data, &[row], &fixture.intent, Some(&fixture.roots())).unwrap();
    assert!(inventory::current(decoded, &fixture.snapshot, fixture.command).is_err());
}

#[test]
fn raw_terminal_state_binds_attempt_generation_previous_action_and_phase() {
    let value = committed();
    for (field, replacement) in [
        ("generation", json!(5)),
        ("active_attempt", json!(Uuid::from_u128(555))),
        ("last_action", json!("drain")),
        ("removal", json!("draining")),
        ("version", json!(6)),
    ] {
        let mut changed = value.clone();
        changed["observed_progress"][field] = replacement;
        assert!(terminal::history_entries(&json!({"terminal":[changed]})).is_err());
    }
    let mut applied = value;
    applied["observed_progress"]["active_attempt"] = Value::Null;
    applied["observed_progress"]["removal"] = json!("deleted");
    applied["observed_progress"]["last_action"] = json!("delete");
    applied["observed_progress"]["version"] = json!(8);
    assert!(terminal::history_entries(&json!({"terminal":[applied]})).is_ok());
    applied["observed_progress"]["last_action"] = json!("detach");
    assert!(terminal::history_entries(&json!({"terminal":[applied]})).is_err());
}
