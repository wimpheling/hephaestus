//! Controlled comparison/writer-input tests; not database or physical evidence.
mod fixture;
use super::{existing, wire};
use fixture::Exact;
use recipe_application::{InstanceClosurePublicationOrigin, ResourceAction};
use serde_json::json;

#[test]
fn full_stored_pending_survives_without_reconstructing_native_fields() {
    let exact = Exact::new();
    let original = &exact.context["pending"][0];
    assert_eq!(
        wire::admitted_pending(&exact.context, 9, &exact.request).unwrap(),
        original
    );
    assert!(wire::admitted_pending(&exact.context, 10, &exact.request).is_err());
    let mut duplicate = exact.context.clone();
    duplicate["pending"]
        .as_array_mut()
        .unwrap()
        .push(original.clone());
    assert!(wire::admitted_pending(&duplicate, 9, &exact.request).is_err());
}

#[test]
fn pending_rejects_each_original_root_substitution() {
    let exact = Exact::new();
    for name in [
        "resource",
        "drain_claim",
        "actor",
        "idempotency",
        "create_claim",
        "create_actor",
        "create_idempotency",
        "prepared",
    ] {
        let mut wrong = exact.context.clone();
        wrong["pending"][0][name] = json!("substituted");
        assert!(
            wire::admitted_pending(&wrong, 9, &exact.request).is_err(),
            "{name}"
        );
    }
    let mut wrong = exact.context.clone();
    wrong["pending"][0]["unknown"] = json!(true);
    assert!(wire::admitted_pending(&wrong, 9, &exact.request).is_err());
}

#[test]
fn native_generation_last_action_and_progress_are_not_guessed() {
    let exact = Exact::new();
    let state = &exact.context["pending"][0]["observed_progress"];
    for (name, value) in [
        ("generation", json!(3)),
        ("version", json!(-1)),
        ("last_action", json!(ResourceAction::Drain)),
        ("active_attempt", json!(null)),
        ("install", json!("pending")),
        ("diagnostic", json!("arbitrary error")),
    ] {
        let mut wrong = state.clone();
        wrong[name] = value;
        assert!(
            wire::progress(&wrong, exact.request.drain()).is_err(),
            "{name}"
        );
    }
    let mut wrong = state.clone();
    wrong.as_object_mut().unwrap().remove("last_action");
    assert!(wire::progress(&wrong, exact.request.drain()).is_err());
}

#[test]
fn positive_tagged_replay_preserves_saved_publisher_and_full_pending() {
    let exact = Exact::new();
    existing::compare(&exact.data, &exact.closure, &exact.target).unwrap();
    for name in [
        "command_id",
        "actor",
        "idempotency",
        "admission_request",
        "publication_request",
        "admission_version",
        "pending",
    ] {
        let mut wrong = exact.data.clone();
        wrong["publication"][name] = json!(null);
        assert!(
            existing::compare(&wrong, &exact.closure, &exact.target).is_err(),
            "{name}"
        );
    }
    let mut wrong = exact.data.clone();
    wrong["publication"]["pending"]["observed_progress"]["version"] = json!(4);
    assert!(existing::compare(&wrong, &exact.closure, &exact.target).is_err());
    wrong = exact.data.clone();
    wrong["expected_version"] = json!(3);
    assert!(existing::compare(&wrong, &exact.closure, &exact.target).is_err());
}

#[test]
fn invalid_or_missing_fresh_tag_never_falls_back_to_original() {
    let exact = Exact::new();
    for tag in [json!(null), json!({"kind":"unknown"})] {
        let mut wrong = exact.data.clone();
        wrong["publication"] = tag;
        assert!(existing::compare(&wrong, &exact.closure, &exact.target).is_err());
    }
    let mut original = exact.closure.clone();
    original.publication_origin = InstanceClosurePublicationOrigin::OriginalDrain;
    original.actor = original.drain.command.actor_id();
    original.request = original.drain.provenance.request_id;
    let mut data = exact.data.clone();
    data.as_object_mut().unwrap().remove("publication");
    data["creator"] = json!(original.actor);
    data["request"] = json!(original.request);
    existing::compare(&data, &original, &exact.target).unwrap();
    data["publication"] = json!({"kind":"unknown"});
    assert!(existing::compare(&data, &original, &exact.target).is_err());
    data.as_object_mut().unwrap().remove("publication");
    data["unknown"] = json!(true);
    assert!(existing::compare(&data, &original, &exact.target).is_err());
}

#[test]
fn current_cleanup_user_is_independent_of_recorded_publisher() {
    let exact = Exact::new();
    let current = recipe_application::CommandIdentity::from_recorded(
        identity_domain::UserId::from_uuid(uuid::Uuid::from_u128(30)),
        identity_domain::RequestId::from_uuid(uuid::Uuid::from_u128(31)),
        recipe_application::DeploymentOperation::Remove,
    )
    .unwrap();
    let request = recipe_application::ExistingInstanceClosureRequest::new(
        current,
        exact.request.intent().clone(),
        exact.target.clone(),
        exact.closure.clone(),
    )
    .unwrap();
    assert_ne!(request.command().actor_id(), request.expected().actor);
    existing::compare(&exact.data, request.expected(), request.target()).unwrap();
    let mut changed = exact.data.clone();
    changed["publication"]["admission_request"] = json!(uuid::Uuid::from_u128(32));
    assert!(existing::compare(&changed, request.expected(), request.target()).is_err());
    changed = exact.data;
    changed["publication"]["publication_request"] = json!(uuid::Uuid::from_u128(33));
    assert!(existing::compare(&changed, request.expected(), request.target()).is_err());
}
