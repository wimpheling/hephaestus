//! Pure native-wire comparisons, not completed transitions or current rights proof.
#[path = "fixture.rs"]
mod fixture;
use super::compare;
use fixture::Exact;
use serde_json::json;
use uuid::Uuid;

#[test]
fn original_and_fresh_replays_keep_saved_publisher_independent_of_current_manager() {
    for fresh in [false, true] {
        let exact = Exact::new(fresh);
        assert_ne!(
            exact.request.command().actor_id(),
            exact.request.expected().actor
        );
        compare(&exact.data, &exact.request).unwrap();
    }
}

#[test]
fn missing_null_unknown_and_partial_publication_never_fall_back() {
    let exact = Exact::new(true);
    let mut missing = exact.data.clone();
    missing.as_object_mut().unwrap().remove("publication");
    assert!(compare(&missing, &exact.request).is_err());
    for value in [json!(null), json!({}), json!({"kind":"unknown"})] {
        let mut wrong = exact.data.clone();
        wrong["publication"] = value;
        assert!(compare(&wrong, &exact.request).is_err());
    }
    let normal = Exact::new(false);
    let mut wrong = normal.data.clone();
    wrong["publication"] = exact.data["publication"].clone();
    assert!(compare(&wrong, &normal.request).is_err());
}

#[test]
fn every_recorded_publication_pin_compares_against_checked_original() {
    let exact = Exact::new(true);
    for (field, value) in [
        ("kind", json!("unknown")),
        ("command_id", json!(Uuid::from_u128(99))),
        ("actor", json!(Uuid::from_u128(99))),
        ("idempotency", json!(Uuid::from_u128(99))),
        ("admission_request", json!(Uuid::from_u128(99))),
        ("publication_request", json!(Uuid::from_u128(99))),
        ("admission_version", json!(8)),
        ("pending", json!({})),
    ] {
        let mut wrong = exact.data.clone();
        wrong["publication"][field] = value;
        assert!(compare(&wrong, &exact.request).is_err(), "{field}");
    }
    let mut wrong = exact.data.clone();
    wrong["publication"]["extra"] = json!(true);
    assert!(compare(&wrong, &exact.request).is_err());
}

#[test]
fn full_pending_create_drain_preparation_and_progress_fields_are_preserved() {
    let exact = Exact::new(true);
    for field in [
        "resource",
        "drain_claim",
        "actor",
        "idempotency",
        "create_claim",
        "create_actor",
        "create_idempotency",
        "prepared",
        "observed_progress",
    ] {
        let mut wrong = exact.data.clone();
        wrong["publication"]["pending"][field] = json!(null);
        assert!(compare(&wrong, &exact.request).is_err(), "{field}");
    }
    for (field, value) in [
        ("generation", json!(3)),
        ("last_action", json!("drain")),
        ("active_attempt", json!(Uuid::from_u128(99))),
        ("version", json!(4)),
        ("removal", json!("detached")),
        ("diagnostic", json!("drain_pending")),
    ] {
        let mut wrong = exact.data.clone();
        wrong["publication"]["pending"]["observed_progress"][field] = value;
        assert!(compare(&wrong, &exact.request).is_err(), "{field}");
    }
    let mut wrong = exact.data.clone();
    wrong["publication"]["pending"]["observed_progress"]["extra"] = json!(true);
    assert!(compare(&wrong, &exact.request).is_err());
}

#[test]
fn all_nine_closure_fields_and_unknown_fields_still_deny_substitution() {
    for fresh in [false, true] {
        let exact = Exact::new(fresh);
        for field in [
            "drain",
            "actor",
            "idempotency",
            "removal_id",
            "command_key",
            "expected_version",
            "creator",
            "request",
            "input_hash",
        ] {
            let mut wrong = exact.data.clone();
            wrong[field] = json!(null);
            assert!(compare(&wrong, &exact.request).is_err(), "{field}");
        }
        let mut wrong = exact.data.clone();
        wrong["extra"] = json!(true);
        assert!(compare(&wrong, &exact.request).is_err());
    }
}

#[test]
fn recorded_recovery_progress_is_compared_without_current_snapshot_reconstruction() {
    let mut exact = Exact::new(true);
    let mut closure = exact.request.expected().clone();
    let recipe_application::InstanceClosurePublicationOrigin::QualifiedFreshRemove(publisher) =
        &closure.publication_origin
    else {
        panic!("fresh fixture");
    };
    {
        let state = &mut exact.data["publication"]["pending"]["observed_progress"];
        state["version"] = json!(4);
        state["removal"] = json!("recovery_required");
        state["diagnostic"] = json!(recipe_application::DiagnosticCode::ProviderOutcomeUnknown);
    }
    assert!(compare(&exact.data, &exact.request).is_err());
    let state =
        super::super::decode_state(&exact.data["publication"]["pending"]["observed_progress"])
            .unwrap();
    let progress = recipe_application::InstanceClosurePublicationProgress::new(
        closure.drain.clone(),
        state.progress().unwrap(),
        2,
        state.last_action,
    )
    .unwrap();
    let publication = recipe_application::QualifiedFreshInstanceClosurePublication::new(
        closure.drain.clone(),
        publisher.command(),
        publisher.native_command_id(),
        publisher.admission_request(),
        publisher.publication_request(),
        publisher.admission_version(),
        progress,
    )
    .unwrap();
    closure.publication_origin =
        recipe_application::InstanceClosurePublicationOrigin::QualifiedFreshRemove(Box::new(
            publication,
        ));
    exact.request = recipe_application::ExistingInstanceClosureRequest::new(
        exact.request.command(),
        exact.request.intent().clone(),
        exact.request.target().clone(),
        closure,
    )
    .unwrap();
    compare(&exact.data, &exact.request).unwrap();
}
