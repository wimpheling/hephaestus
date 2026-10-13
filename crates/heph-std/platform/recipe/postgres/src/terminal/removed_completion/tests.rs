//! Controlled comparison rows; no native admission, cleanup or result is fabricated.
use super::{resources, rows};
use crate::{execution_transitions, rows::ReceiptRow};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentId, DeploymentIntent, DeploymentKey,
    DeploymentLifecycle, DeploymentOperation, DeploymentSnapshot, InstallProgress, RemovalProgress,
    ResourceProgress,
};
use release_domain::ContentHash;
use uuid::Uuid;

fn fixture() -> (DeploymentSnapshot, rows::Finished) {
    let source = "contract_version=1\nrecipe_id=\"data\"\nrecipe_version=\"1\"\n[[resources]]\nkind=\"volume\"\nname=\"data\"\nsource={type=\"created\",capacity_bytes={source=\"literal\",value=16777216}}\n";
    let parsed = recipe_domain::parse_recipe(source.as_bytes()).unwrap();
    let resolved = parsed
        .resolve(
            &std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
            &[],
        )
        .unwrap();
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        forge_domain::ProjectId::from_uuid(Uuid::new_v4()),
        DeploymentKey::parse("finished").unwrap(),
        &parsed,
        &resolved,
    )
    .unwrap();
    let progress = ResourceProgress {
        install: InstallProgress::Ready,
        removal: RemovalProgress::Retained,
        version: 4,
        active_attempt: None,
        diagnostic: None,
    };
    let resources = intent
        .resources()
        .keys()
        .map(|name| (name.clone(), progress.clone()))
        .collect();
    let command = CommandIdentity::from_recorded(
        UserId::new(),
        RequestId::new(),
        DeploymentOperation::Remove,
    )
    .unwrap();
    let input = ContentHash::digest(b"actual-recorded-original-remove-input");
    let fingerprint =
        ContentHash::digest(&serde_json::to_vec(&(command, intent.id(), 7_u64)).unwrap());
    let row = rows::Finished {
        receipt: ReceiptRow {
            id: command.id().as_uuid(),
            deployment_id: intent.id().as_uuid(),
            project_id: intent.project_id().as_uuid(),
            input_hash: input.as_bytes().to_vec(),
            receipt_lifecycle: "removed".into(),
            receipt_version: 8,
            event_id: Uuid::new_v4(),
            event_cursor: 10,
            event_aggregate_version: 6,
        },
        finish_fingerprint: fingerprint.as_bytes().to_vec(),
        transition_id: execution_transitions::transition_id(command.id().as_uuid(), "finished"),
        actor_id: command.actor_id().as_uuid(),
        request_id: Uuid::new_v4(),
        command_actor: Some(command.actor_id().as_uuid()),
        idempotency_id: Some(command.idempotency_id().as_uuid()),
        command_input: Some(input.as_bytes().to_vec()),
        before_version: Some(7),
        after_version: Some(8),
        evidence_valid: true,
    };
    (
        DeploymentSnapshot {
            intent,
            lifecycle: DeploymentLifecycle::Removed,
            version: 8,
            resources,
        },
        row,
    )
}

#[test]
fn returns_saved_before_cas_and_rejects_current_version_substitution() {
    let (mut snapshot, mut row) = fixture();
    let (receipt, before) = rows::restore(&row, &snapshot).unwrap();
    assert_eq!(before, 7);
    assert_eq!(receipt.deployment_version, 8);
    assert_eq!(receipt.command.actor_id().as_uuid(), row.actor_id);
    row.before_version = Some(6);
    assert!(rows::restore(&row, &snapshot).is_err());
    row.before_version = Some(7);
    snapshot.version = 9;
    assert!(rows::restore(&row, &snapshot).is_err());
    snapshot.version = 8;
    row.finish_fingerprint = ContentHash::digest(b"another-finish-input")
        .as_bytes()
        .to_vec();
    assert!(rows::restore(&row, &snapshot).is_err());
}

#[test]
fn coherent_foreign_or_missing_provenance_cannot_restore_original() {
    let (snapshot, mut row) = fixture();
    row.receipt.project_id = Uuid::new_v4();
    assert!(matches!(
        rows::restore(&row, &snapshot),
        Err(DeploymentError::InputConflict)
    ));
    row.receipt.project_id = snapshot.intent.project_id().as_uuid();
    row.evidence_valid = false;
    assert!(rows::restore(&row, &snapshot).is_err());
    row.evidence_valid = true;
    row.command_actor = Some(Uuid::new_v4());
    assert!(rows::restore(&row, &snapshot).is_err());
    row.command_actor = Some(row.actor_id);
    row.request_id = Uuid::nil();
    assert!(rows::restore(&row, &snapshot).is_err());
    row.request_id = Uuid::new_v4();
    row.command_input = None;
    assert!(rows::restore(&row, &snapshot).is_err());
}

#[test]
fn none_requires_unfinished_state_without_contradictory_or_duplicate_result() {
    let (mut snapshot, row) = fixture();
    assert!(rows::select(&[], &snapshot).is_err());
    snapshot.lifecycle = DeploymentLifecycle::Removing;
    assert!(rows::select(&[], &snapshot).unwrap().is_none());
    assert!(rows::select(&[row], &snapshot).is_err());
    snapshot.lifecycle = DeploymentLifecycle::Removed;
    let (_, row1) = fixture();
    let (_, row2) = fixture();
    assert!(rows::select(&[row1, row2], &snapshot).is_err());
}

#[test]
fn terminal_policy_requires_exact_disposition_and_no_owned_operation() {
    let (mut snapshot, _) = fixture();
    assert!(resources::policy(&snapshot).is_ok());
    let state = snapshot.resources.values_mut().next().unwrap();
    state.removal = RemovalProgress::Deleted;
    assert!(resources::policy(&snapshot).is_err());
    let state = snapshot.resources.values_mut().next().unwrap();
    state.removal = RemovalProgress::Retained;
    state.active_attempt =
        Some(recipe_application::DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap());
    assert!(resources::policy(&snapshot).is_err());
    snapshot.resources.clear();
    assert!(resources::policy(&snapshot).is_err());
}
