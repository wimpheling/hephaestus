//! Pure boundary denials; these are not database/native terminal evidence.
use super::{decode, wire};
use recipe_application::DeploymentError;
use serde_json::{Value, json};
use uuid::Uuid;

fn detached(request: &Value) -> Value {
    json!({"version":1,"request":request,"birth_proven":true,"cleanup_ready":true,
        "removal_id":Uuid::from_u128(7),"inventory":{
          "runs":[],"leases":[],"mount_grants":[],"volumes":[],"consumers":[],"work":[]}})
}
#[test]
fn detach_wire_rejects_substituted_request_and_extra_or_missing_native_fields() {
    let request = json!({"original":"comparison"});
    let original = detached(&request);
    decode::detach(&original, &request).unwrap();
    let mut wrong = original.clone();
    wrong["request"]["original"] = json!("other");
    assert!(decode::detach(&wrong, &request).is_err());
    let mut wrong = original.clone();
    wrong["unknown"] = json!(true);
    assert!(decode::detach(&wrong, &request).is_err());
    for name in ["removal_id", "inventory", "cleanup_ready", "birth_proven"] {
        let mut wrong = original.clone();
        wrong.as_object_mut().unwrap().remove(name);
        assert!(decode::detach(&wrong, &request).is_err(), "{name}");
    }
}
#[test]
fn detach_wire_never_turns_missing_or_held_inventory_into_drained() {
    let request = json!({"original":"comparison"});
    let original = detached(&request);
    for (name, value) in [
        ("birth_proven", json!(false)),
        ("cleanup_ready", json!(false)),
        ("removal_id", json!(Uuid::nil())),
        ("version", json!(2)),
        ("inventory", json!(null)),
    ] {
        let mut wrong = original.clone();
        wrong[name] = value;
        assert!(decode::detach(&wrong, &request).is_err(), "{name}");
    }
    let mut wrong = original;
    wrong["inventory"]["work"] = json!([{"kind":"unknown","id":Uuid::from_u128(1)}]);
    assert!(decode::detach(&wrong, &request).is_err());
}
#[test]
fn terminal_tuple_requires_one_actual_nonnil_exact_successor() {
    let operation = Uuid::from_u128(9);
    assert_eq!(
        decode::terminal(&[(operation, 8)], 7).unwrap(),
        (operation, 8)
    );
    for rows in [
        vec![],
        vec![(operation, 8), (operation, 8)],
        vec![(Uuid::nil(), 8)],
        vec![(operation, -1)],
        vec![(operation, 7)],
        vec![(operation, 9)],
    ] {
        assert!(decode::terminal(&rows, 7).is_err());
    }
    assert!(matches!(
        decode::terminal(&[(operation, i64::MAX)], i64::MAX.cast_unsigned()),
        Err(DeploymentError::StaleClaim)
    ));
}
#[test]
fn full_configuration_bytes_preserve_volume_identity_and_root() {
    let scope =
        vm_trait::VmProviderOwnerScope::new("terminal-provider".into(), "host".into()).unwrap();
    let make = |root: &str, namespace: u128| {
        release_service::InstanceExecutionConfiguration::new(
            scope.clone(),
            root.into(),
            volume_domain::VolumeRootNamespaceId::from_uuid(Uuid::from_u128(namespace)).unwrap(),
            release_domain::RuntimePolicy {
                vcpus: 1,
                memory_mib: 128,
                network: release_domain::NetworkAccess::Disabled,
            },
            "platform/v1".into(),
        )
        .unwrap()
    };
    let config = make("/var/tmp/original-volumes", 1);
    let encoded = wire::configuration_bytes(&config).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&encoded).unwrap(),
        config.canonical_json()
    );
    assert_ne!(
        encoded,
        wire::configuration_bytes(&make("/var/tmp/replacement-volumes", 1)).unwrap()
    );
    assert_ne!(
        encoded,
        wire::configuration_bytes(&make("/var/tmp/original-volumes", 2)).unwrap()
    );
}
