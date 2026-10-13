use super::{AgentRow, authored_volume_slots};
use crate::release::{ReleaseAgent, ReleaseError};
use release_domain::{GuestMountPath, VolumeAccessMode, VolumeSlotDeclaration};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

fn row(contract: serde_json::Value, requires_state: bool) -> AgentRow {
    AgentRow {
        id: Uuid::from_u128(1),
        family_id: Uuid::from_u128(2),
        agent_key: "reviewer".to_owned(),
        display_name: "Reviewer".to_owned(),
        runtime_contract: contract,
        parameter_schema: json!([]),
        secret_slot_schema: json!([{
            "key":"model", "purpose":"model", "required":true,
            "delivery_modes":["brokered"], "phases":["normal"], "destinations":[]
        }]),
        requires_state,
        update_hook: None,
        created_at: OffsetDateTime::UNIX_EPOCH,
    }
}

fn slot_json(name: &str, path: &str) -> serde_json::Value {
    json!({"slot":name,"guest_path":path,"access_mode":"read_write","required":true,"minimum_capacity_bytes":16_777_216})
}

#[test]
fn projection_preserves_authored_provenance_and_lifts_legacy_without_rewriting() {
    let legacy = json!({"policy_ceiling":{"vcpus":1,"memory_mib":512,"network":"disabled"}});
    let agent = ReleaseAgent::try_from(row(legacy.clone(), true)).expect("legacy projection");
    assert!(agent.volume_slots.is_empty());
    let effective = agent
        .effective_volume_slots()
        .expect("effective legacy scope");
    assert_eq!(effective.len(), 1);
    assert_eq!(effective[0].slot().as_str(), "state");
    assert_eq!(effective[0].guest_path().as_str(), "/var/lib/hephaestus");
    assert_eq!(effective[0].access_mode(), VolumeAccessMode::ReadWrite);
    assert_eq!(effective[0].minimum_capacity_bytes(), 1);
    assert_eq!(agent.required_secret_slot_count(), 1);
    assert!(legacy.get("volume_slots").is_none());
    let mut explicit = legacy;
    explicit["volume_slots"] = json!([slot_json("data", "/data")]);
    let agent = ReleaseAgent::try_from(row(explicit, true)).expect("explicit projection");
    assert_eq!(agent.volume_slots.len(), 1);
    assert_eq!(agent.effective_volume_slots().expect("effective").len(), 2);
}

#[test]
fn malformed_stored_volume_declarations_fail_closed() {
    for slots in [
        json!(null),
        json!({}),
        json!([slot_json("data", "/release")]),
        json!([slot_json("data", "/data"), slot_json("data", "/other")]),
        json!([slot_json("state", "/data")]),
        json!([slot_json("data", "/var/lib/hephaestus/cache")]),
    ] {
        assert!(matches!(
            authored_volume_slots(&json!({"volume_slots":slots}), true),
            Err(ReleaseError::InvalidStoredData)
        ));
    }
    let mut unknown = slot_json("data", "/data");
    unknown["host_path"] = json!("/tmp/escape");
    assert!(authored_volume_slots(&json!({"volume_slots":[unknown]}), false).is_err());
}

#[test]
fn typed_authoritative_projection_keeps_full_declared_scope() {
    let slots = authored_volume_slots(&json!({"volume_slots":[slot_json("data", "/data")]}), false)
        .expect("slots");
    assert_eq!(
        slots,
        vec![
            VolumeSlotDeclaration::new(
                serde_json::from_value(json!("data")).expect("slot key"),
                GuestMountPath::parse("/data").expect("path"),
                VolumeAccessMode::ReadWrite,
                true,
                16_777_216,
            )
            .expect("slot")
        ]
    );
}
