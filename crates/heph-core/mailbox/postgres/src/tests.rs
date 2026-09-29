use super::{
    MailboxPersistenceError,
    helpers::{deterministic_transport_id, validate_payload},
};
use mailbox_domain::{BodyReference, BodyReferenceId, ContentMetadata};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn content(body: &[u8], encoding: Option<&str>) -> ContentMetadata {
    ContentMetadata::new(
        BodyReference::new(
            BodyReferenceId::from_uuid(Uuid::new_v4()),
            u32::try_from(body.len()).expect("bounded test body"),
            Sha256::digest(body).into(),
        )
        .expect("bounded body reference"),
        Some(String::from("application/octet-stream")),
        encoding.map(str::to_owned),
    )
    .expect("valid content metadata")
}

#[test]
fn accepts_empty_identity_payload() {
    let content = content(&[], Some("identity"));
    assert!(validate_payload(&content, &[], 0).is_ok());
}

#[test]
fn rejects_mismatched_payload_evidence_before_persistence() {
    let content = content(b"expected", None);
    assert!(matches!(
        validate_payload(&content, b"different", 9),
        Err(MailboxPersistenceError::PayloadIntegrity)
    ));
    assert!(matches!(
        validate_payload(&content, b"expected", 7),
        Err(MailboxPersistenceError::PayloadIntegrity)
    ));
}

#[test]
fn rejects_non_identity_content_encoding_before_persistence() {
    let content = content(b"body", Some("gzip"));
    assert!(matches!(
        validate_payload(&content, b"body", 4),
        Err(MailboxPersistenceError::UnsupportedContentEncoding)
    ));
}

#[test]
fn transport_id_replay_is_stable_and_new_wake_is_distinct() {
    let wake = Uuid::from_u128(1);
    let dispatch = Uuid::from_u128(2);
    assert_eq!(
        deterministic_transport_id(wake, dispatch),
        deterministic_transport_id(wake, dispatch)
    );
    assert_ne!(
        deterministic_transport_id(wake, dispatch),
        deterministic_transport_id(Uuid::from_u128(3), dispatch)
    );
}
