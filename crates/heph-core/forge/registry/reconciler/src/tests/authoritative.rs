use super::support::*;
use crate::*;

#[tokio::test]
async fn missed_event_is_found_by_full_authoritative_reconciliation() {
    let value = verified_intent().approve().expect("approved");
    let reducer = reconciler(vec![value], ZotInspection::Missing);
    let result = reducer.reconcile_all().await.expect("reconciled");
    assert!(matches!(
        result.intents[0].actions.as_slice(),
        [ReconciliationAction::MarkMissing { .. }]
    ));
}

#[tokio::test]
async fn exact_graph_recovers_missing_content_without_a_new_approval() {
    let value = verified_intent()
        .approve()
        .expect("approved")
        .mark_missing()
        .expect("missing");
    let reducer = reconciler(vec![value], inspection());
    let result = reducer.reconcile_all().await.expect("reconciled");
    assert!(matches!(
        result.intents[0].actions.as_slice(),
        [ReconciliationAction::RestoreVerified { .. }]
    ));
}

#[tokio::test]
async fn missing_approved_content_fails_closed() {
    let value = verified_intent().approve().expect("approved");
    let reducer = reconciler(vec![value.clone()], ZotInspection::Missing);
    let result = reducer
        .reduce_claimed(&claim(Some(value.claim().namespace().clone()), None))
        .await
        .expect("reduced");
    assert!(matches!(
        result.intents[0].actions.as_slice(),
        [ReconciliationAction::MarkMissing { .. }]
    ));
}

#[tokio::test]
async fn descriptor_inconsistency_marks_approved_content_missing() {
    let value = verified_intent().approve().expect("approved");
    let reducer = reconciler(vec![value], descriptor_mismatch_inspection());
    let result = reducer.reconcile_all().await.expect("reconciled");
    assert!(matches!(
        result.intents[0].actions.as_slice(),
        [ReconciliationAction::MarkMissing {
            reason: Inconsistency::ManifestDescriptorMismatch,
            ..
        }]
    ));
}

#[tokio::test]
async fn unknown_namespace_is_rejected_without_reading_zot() {
    let reducer = reconciler(Vec::new(), inspection());
    let result = reducer
        .reduce_claimed(&claim(None, None))
        .await
        .expect("reduced");
    assert_eq!(
        result.completion,
        NotificationCompletion::Rejected {
            failure_code: "unknown_namespace".to_owned()
        }
    );
    assert!(matches!(
        result.actions.as_slice(),
        [ReconciliationAction::OrphanNamespace { .. }]
    ));
}
