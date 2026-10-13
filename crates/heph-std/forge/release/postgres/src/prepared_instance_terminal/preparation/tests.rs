//! Controlled transport substitutions only, not native or physical evidence.
use super::decode;
use serde_json::{Value, json};
use uuid::Uuid;

fn transport() -> Value {
    json!({"version":1,"project":Uuid::from_u128(1),"deployment":Uuid::from_u128(2),
        "intent_hash":[1,2],"resource":"writer","create_claim":{"comparison":"Create"},
        "prepared":{"comparison":"original import"},"closure":{"comparison":"native rows"},
        "delete_or_detach_claim":{"comparison":"Delete"},
        "current_command":{"comparison":"actual current Remove"},
        "configuration":{"comparison":"whole config"},"before_instance_version":null})
}
fn reply(input: &Value, version: u64) -> Value {
    let mut request = input.clone();
    request["before_instance_version"] = json!(version);
    json!({"version":1,"request":request,"history":{
        "resource":input["resource"],"delete_claim":input["delete_or_detach_claim"],
        "before_instance_version":version,"terminal_operation_id":null,
        "after_instance_version":null,"publisher":null,"observed_progress":{
            "install":"ready","removal":"detached","version":8,
            "active_attempt":Uuid::from_u128(3),"diagnostic":null,"generation":4,"last_action":"detach"}}})
}
#[test]
fn native_preparation_cannot_substitute_any_represented_original_or_current_command() {
    let transport = transport();
    let original = reply(&transport, 7);
    assert_eq!(decode::continuation(&original, &transport).unwrap(), 7);
    for name in [
        "project",
        "deployment",
        "intent_hash",
        "resource",
        "create_claim",
        "prepared",
        "closure",
        "delete_or_detach_claim",
        "current_command",
        "configuration",
        "version",
    ] {
        let mut wrong = original.clone();
        wrong["request"][name] = json!("substituted");
        assert!(decode::continuation(&wrong, &transport).is_err(), "{name}");
    }
    let mut wrong = original.clone();
    wrong["request"]["extra"] = json!(true);
    assert!(decode::continuation(&wrong, &transport).is_err());
    let mut wrong = original;
    wrong["history"]["delete_claim"] = json!({"comparison":"foreign"});
    assert!(decode::continuation(&wrong, &transport).is_err());
}
#[test]
fn no_null_zero_overflow_or_inconsistent_native_cas_is_a_checked_request() {
    let transport = transport();
    for value in [
        json!(null),
        json!(0),
        json!(-1),
        json!(i64::MAX),
        json!(i64::MAX.cast_unsigned() + 1),
        json!("7"),
    ] {
        let mut wrong = reply(&transport, 7);
        wrong["request"]["before_instance_version"] = value;
        assert!(decode::continuation(&wrong, &transport).is_err());
    }
    let mut wrong = reply(&transport, 7);
    wrong["history"]["before_instance_version"] = json!(8);
    assert!(decode::continuation(&wrong, &transport).is_err());
    let mut saved = reply(&transport, 7);
    saved["history"]["terminal_operation_id"] = json!(Uuid::from_u128(9));
    saved["history"]["after_instance_version"] = json!(8);
    saved["history"]["publisher"] = json!({"command_id":Uuid::from_u128(10),"actor_id":Uuid::from_u128(11),
        "request_id":Uuid::from_u128(12),"admission_command_id":null});
    assert_eq!(decode::continuation(&saved, &transport).unwrap(), 7);
    saved["history"]["terminal_operation_id"] = json!(Uuid::nil());
    assert!(decode::continuation(&saved, &transport).is_err());
}
#[test]
fn unknown_history_and_unqualified_partial_publication_shapes_are_denied() {
    let transport = transport();
    let original = reply(&transport, 7);
    for (name, value) in [
        ("extra", json!(true)),
        ("after_instance_version", json!(8)),
        ("publisher", json!({"actor_id":Uuid::from_u128(9)})),
    ] {
        let mut wrong = original.clone();
        wrong["history"][name] = value;
        assert!(decode::continuation(&wrong, &transport).is_err(), "{name}");
    }
    for (name, value) in [
        ("generation", json!(0)),
        ("active_attempt", json!(Uuid::nil())),
        ("install", json!("unknown")),
        ("unknown", json!(true)),
    ] {
        let mut wrong = original.clone();
        wrong["history"]["observed_progress"][name] = value;
        assert!(decode::continuation(&wrong, &transport).is_err(), "{name}");
    }
}
