use super::support::*;
use crate::*;
use registry_domain::OciMediaType;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn duplicate_observations_propose_the_same_idempotent_verification() {
    let value = intent();
    let reducer = reconciler(vec![value.clone()], inspection());
    let target = ObservedTarget {
        digest: digest('a'),
        media_type: OciMediaType::parse("application/vnd.oci.image.index.v1+json")
            .expect("media type"),
    };
    let first = reducer
        .reduce_claimed(&claim(
            Some(value.claim().namespace().clone()),
            Some(target.clone()),
        ))
        .await
        .expect("first");
    let second = reducer
        .reduce_claimed(&claim(
            Some(value.claim().namespace().clone()),
            Some(target),
        ))
        .await
        .expect("second");
    assert_eq!(first, second);
    assert!(matches!(
        first.intents[0].actions.as_slice(),
        [ReconciliationAction::RecordVerified { .. }]
    ));
}

#[tokio::test]
async fn process_next_completes_the_claim_but_never_executes_its_action() {
    let value = intent();
    let inbox = FakeInbox::default();
    inbox
        .claimed
        .lock()
        .expect("not poisoned")
        .push_back(claim(Some(value.claim().namespace().clone()), None));
    let reducer = RegistryReconciler::new(
        inbox.clone(),
        FakeIntents {
            values: vec![value],
        },
        FakeZot {
            value: inspection(),
        },
    );
    let result = reducer
        .process_next(Duration::from_secs(30))
        .await
        .expect("processed")
        .expect("claim");
    assert!(matches!(
        result.intents[0].actions.as_slice(),
        [ReconciliationAction::RecordVerified { .. }]
    ));
    assert_eq!(
        inbox.completions.lock().expect("not poisoned").as_slice(),
        [NotificationCompletion::Processed]
    );
}

#[tokio::test]
async fn production_processing_leaves_the_claim_retryable_when_action_application_fails() {
    let value = intent();
    let inbox = FakeInbox::default();
    inbox
        .claimed
        .lock()
        .expect("not poisoned")
        .push_back(claim(Some(value.claim().namespace().clone()), None));
    let reducer = RegistryReconciler::new(
        inbox.clone(),
        FakeIntents {
            values: vec![value],
        },
        FakeZot {
            value: inspection(),
        },
    );
    let executor = FakeExecutor {
        fail: true,
        applied: Arc::default(),
    };
    assert!(
        reducer
            .process_next_and_apply(Duration::from_secs(30), &executor)
            .await
            .is_err()
    );
    assert!(inbox.completions.lock().expect("not poisoned").is_empty());
}

#[tokio::test]
async fn reordered_observation_never_overrides_the_authoritative_digest() {
    let value = verified_intent().approve().expect("approved");
    let reducer = reconciler(vec![value.clone()], inspection());
    let old_target = ObservedTarget {
        digest: digest('f'),
        media_type: OciMediaType::parse("application/vnd.oci.image.index.v1+json")
            .expect("media type"),
    };
    let reduction = reducer
        .reduce_claimed(&claim(
            Some(value.claim().namespace().clone()),
            Some(old_target),
        ))
        .await
        .expect("reduced");
    assert!(matches!(
        reduction.actions.as_slice(),
        [ReconciliationAction::ObservedDifferentTarget { .. }]
    ));
    assert!(reduction.intents[0].actions.is_empty());
}
