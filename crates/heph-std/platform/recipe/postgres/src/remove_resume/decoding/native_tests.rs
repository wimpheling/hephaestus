//! Native wire/model substitutions and replay selection, not PG or physical proof.
#[path = "native_fixture.rs"]
mod fixture;
use super::{decode, inventory, replay, wire};
use fixture::Fixture;
use serde_json::json;

#[test]
fn pending_binds_full_originals_and_current_snapshot_without_rebuilding_history() {
    let fixture = Fixture::new();
    let value = fixture.data();
    let decoded = decode(&value, &[], &fixture.intent, Some(&fixture.roots())).unwrap();
    let current = inventory::current(decoded, &fixture.snapshot, fixture.command).unwrap();
    assert_eq!(current.pending.len(), 1);
    assert_eq!(current.pending[0].snapshot(), &fixture.snapshot);
    assert_eq!(current.pending[0].drain(), &fixture.drain);
    assert_eq!(
        current.pending[0].target().prepared(),
        &fixture.preparation.instances()[0]
    );
    assert_eq!(value, fixture.data());
}

#[test]
fn closed_context_and_optional_arrays_reject_unknown_empty_null_and_duplicate_endpoints() {
    let fixture = Fixture::new();
    for field in ["pending", "recovery"] {
        for replacement in [json!([]), json!(null), json!({})] {
            let mut value = fixture.data();
            value[field] = replacement;
            assert!(decode(&value, &[], &fixture.intent, Some(&fixture.roots())).is_err());
        }
    }
    let mut value = fixture.data();
    value["extra"] = json!(true);
    assert!(wire::context(&value, 1).is_err());
    value = fixture.data();
    value["pending"] = json!([fixture.pending.clone(), fixture.pending]);
    assert!(wire::context(&value, 1).is_err());
}

#[test]
fn full_create_preparation_generation_last_action_and_progress_substitutions_deny() {
    let fixture = Fixture::new();
    for (field, replacement) in [
        ("generation", json!(3)),
        ("active_attempt", json!(null)),
        ("last_action", json!("drain")),
        ("version", json!(0)),
        ("diagnostic", json!("unrecognized")),
        ("extra", json!(true)),
    ] {
        let mut value = fixture.data();
        value["pending"][0]["observed_progress"][field] = replacement;
        assert!(
            decode(&value, &[], &fixture.intent, Some(&fixture.roots())).is_err(),
            "{field}"
        );
    }
    for field in [
        "create_actor",
        "create_idempotency",
        "prepared",
        "create_claim",
    ] {
        let mut value = fixture.data();
        value["pending"][0][field] = json!(null);
        assert!(
            decode(&value, &[], &fixture.intent, Some(&fixture.roots())).is_err(),
            "{field}"
        );
    }
    let mut snapshot = fixture.snapshot.clone();
    snapshot
        .resources
        .get_mut(&fixture.drain.resource)
        .unwrap()
        .version += 1;
    let decoded = decode(
        &fixture.data(),
        &[],
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    assert!(inventory::current(decoded, &snapshot, fixture.command).is_err());
}

#[test]
fn advanced_pending_recovery_requires_exact_recovery_lifecycle_and_true_receipt() {
    let mut fixture = Fixture::new();
    fixture.recover(5);
    fixture.pending["observed_progress"] = serde_json::to_value(&fixture.state).unwrap();
    let value = fixture.data();
    let decoded = decode(&value, &[], &fixture.intent, Some(&fixture.roots())).unwrap();
    assert!(replay::receipt("recovery_required", &decoded).is_ok());
    assert!(replay::receipt("removing", &decoded).is_err());
    assert!(replay::receipt("removed", &decoded).is_err());
    let current = inventory::current(decoded, &fixture.snapshot, fixture.command).unwrap();
    assert_eq!(current.pending[0].snapshot(), &fixture.snapshot);
    let mut wrong = fixture.snapshot.clone();
    wrong.lifecycle = recipe_application::DeploymentLifecycle::Removing;
    let decoded = decode(&value, &[], &fixture.intent, Some(&fixture.roots())).unwrap();
    assert!(inventory::current(decoded, &wrong, fixture.command).is_err());
}

#[test]
fn recorded_publisher_is_distinct_from_current_cleanup_and_all_tag_pins_are_checked() {
    let fixture = Fixture::new();
    let row = fixture.closed();
    let value = fixture.closed_data(&row);
    let decoded = decode(
        &value,
        std::slice::from_ref(&row),
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    let current_command = fixture::command(90, 91, recipe_application::DeploymentOperation::Remove);
    let current = inventory::current(decoded, &fixture.snapshot, current_command).unwrap();
    let recipe_application::InstanceClosurePublicationOrigin::QualifiedFreshRemove(publication) =
        &current.closures[0].publication_origin
    else {
        panic!("explicit tag")
    };
    assert_eq!(publication.command(), fixture.command);
    assert_ne!(publication.command(), current_command);
    assert_ne!(
        publication.admission_request(),
        publication.publication_request()
    );
    for (field, replacement) in [
        ("kind", json!("unknown")),
        ("command_id", json!(uuid::Uuid::from_u128(999))),
        ("admission_request", json!(uuid::Uuid::nil())),
        ("publication_request", json!(uuid::Uuid::nil())),
        ("admission_version", json!(u64::MAX)),
        ("pending", json!({})),
        ("actor", json!(uuid::Uuid::from_u128(99))),
    ] {
        let mut wrong = fixture.closed();
        wrong.source["publication"][field] = replacement;
        assert!(
            decode(
                &fixture.closed_data(&wrong),
                std::slice::from_ref(&wrong),
                &fixture.intent,
                Some(&fixture.roots())
            )
            .is_err(),
            "{field}"
        );
    }
}

#[test]
fn closed_recovery_preserves_recorded_publication_and_compares_current_full_state() {
    let mut fixture = Fixture::new();
    let row = fixture.closed();
    fixture.recover(5);
    let mut value = fixture.closed_data(&row);
    value["recovery"] = fixture.recovery();
    let decoded = decode(
        &value,
        std::slice::from_ref(&row),
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    assert!(replay::receipt("recovery_required", &decoded).is_ok());
    let current = inventory::current(
        decoded,
        &fixture.snapshot,
        fixture::command(90, 91, recipe_application::DeploymentOperation::Remove),
    )
    .unwrap();
    assert!(current.pending.is_empty());
    assert_eq!(current.closures.len(), 1);
    for mutation in [0, 1, 2, 3] {
        let mut wrong = value.clone();
        match mutation {
            0 => {
                wrong["recovery"][0]["resource"] = json!("foreign");
            }
            1 => {
                wrong["recovery"][0]["observed_progress"]["generation"] = json!(3);
            }
            2 => {
                wrong["recovery"][0]["extra"] = json!(true);
            }
            _ => {
                wrong["pending"] = json!([fixture.pending]);
            }
        }
        assert!(
            decode(
                &wrong,
                std::slice::from_ref(&row),
                &fixture.intent,
                Some(&fixture.roots())
            )
            .is_err()
        );
    }
}

#[test]
fn replay_selects_a_positive_published_winner_but_never_refreshes_unpublished_cas() {
    let mut fixture = Fixture::new();
    let saved_value = fixture.data();
    let saved = decode(&saved_value, &[], &fixture.intent, Some(&fixture.roots())).unwrap();
    let unchanged = decode(&saved_value, &[], &fixture.intent, Some(&fixture.roots())).unwrap();
    assert!(replay::compare(&saved, &unchanged, &fixture.snapshot, 7).is_ok());
    let row = fixture.closed();
    let winner_value = fixture.closed_data(&row);
    let winner = decode(
        &winner_value,
        std::slice::from_ref(&row),
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    // Selection alone is NOT acceptance: production observe() must independently
    // query the protected loader with FIRST admission/current principal before return.
    assert!(replay::compare(&saved, &winner, &fixture.snapshot, 7).is_ok());
    fixture.recover(5);
    fixture.pending["observed_progress"] = serde_json::to_value(&fixture.state).unwrap();
    let advanced = decode(
        &fixture.data(),
        &[],
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    assert!(replay::compare(&saved, &advanced, &fixture.snapshot, 7).is_err());
    assert_eq!(saved_value["pending"][0]["observed_progress"]["version"], 3);
    let missing = decode(
        &json!({"claims":[],"closures":[]}),
        &[],
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    assert!(replay::compare(&saved, &missing, &fixture.snapshot, 7).is_err());
}

#[test]
fn saved_closed_recovery_endpoint_is_not_equated_to_later_current_progress() {
    let mut fixture = Fixture::new();
    let row = fixture.closed();
    fixture.recover(5);
    let mut saved_value = fixture.closed_data(&row);
    saved_value["recovery"] = fixture.recovery();
    let saved = decode(
        &saved_value,
        std::slice::from_ref(&row),
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    fixture.recover(6);
    let mut current_value = fixture.closed_data(&row);
    current_value["recovery"] = fixture.recovery();
    let current = decode(
        &current_value,
        std::slice::from_ref(&row),
        &fixture.intent,
        Some(&fixture.roots()),
    )
    .unwrap();
    assert!(replay::receipt("recovery_required", &saved).is_ok());
    assert!(replay::compare(&saved, &current, &fixture.snapshot, 8).is_ok());
    let current = inventory::current(
        current,
        &fixture.snapshot,
        fixture::command(90, 91, recipe_application::DeploymentOperation::Remove),
    )
    .unwrap();
    assert_eq!(current.closures[0].drain, fixture.drain);
    assert_eq!(
        saved_value["recovery"][0]["observed_progress"]["version"],
        5
    );
    assert_eq!(
        current_value["recovery"][0]["observed_progress"]["version"],
        6
    );
}

#[path = "native143_tests.rs"]
mod native143;
