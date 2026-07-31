//! Provider-neutral mutation receipt port contract tests.

use async_trait::async_trait;
use event_application::{CommittedMutation, MutationReceiptError, MutationReceiptReader};
use identity_domain::{RequestId, UserId};
use std::sync::Mutex;
use uuid::Uuid;

type ReceiptLookup = (RequestId, UserId, String, String);

struct FakeReader {
    expected: ReceiptLookup,
    receipt: CommittedMutation,
    observed: Mutex<Vec<ReceiptLookup>>,
}

#[async_trait]
impl MutationReceiptReader for FakeReader {
    async fn load(
        &self,
        occurrence_id: RequestId,
        actor_id: UserId,
        aggregate_type: &str,
        primary_scope_kind: &str,
    ) -> Result<CommittedMutation, MutationReceiptError> {
        let lookup = (
            occurrence_id,
            actor_id,
            aggregate_type.to_owned(),
            primary_scope_kind.to_owned(),
        );
        self.observed
            .lock()
            .expect("fake receipt observations")
            .push(lookup.clone());
        if lookup == self.expected {
            Ok(self.receipt.clone())
        } else {
            Err(MutationReceiptError::Missing)
        }
    }
}

#[tokio::test]
async fn port_is_object_safe_and_preserves_the_exact_lookup_identity() {
    let occurrence_id = RequestId::new();
    let actor_id = UserId::new();
    let expected = (
        occurrence_id,
        actor_id,
        String::from("run"),
        String::from("run"),
    );
    let receipt = CommittedMutation {
        event_id: Uuid::new_v4(),
        scope_kind: String::from("run"),
        scope_id: Uuid::new_v4(),
        cursor: 7,
        aggregate_version: 3,
    };
    let fake = FakeReader {
        expected: expected.clone(),
        receipt: receipt.clone(),
        observed: Mutex::new(Vec::new()),
    };
    let reader: &dyn MutationReceiptReader = &fake;

    assert_eq!(
        reader
            .load(occurrence_id, actor_id, "run", "run")
            .await
            .expect("matching receipt"),
        receipt
    );
    assert!(matches!(
        reader
            .load(occurrence_id, actor_id, "release", "repository")
            .await,
        Err(MutationReceiptError::Missing)
    ));
    assert_eq!(
        *fake.observed.lock().expect("fake receipt observations"),
        vec![
            expected,
            (
                occurrence_id,
                actor_id,
                String::from("release"),
                String::from("repository")
            )
        ]
    );
}
