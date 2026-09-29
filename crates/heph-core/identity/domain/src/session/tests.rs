use super::super::{AuthenticatedIdentity, RequestId, UserId};
use super::*;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[test]
fn session_id_is_redacted_except_for_explicit_protocol_serialization() {
    let id = BrowserSessionSid::new();
    let protocol = id.to_protocol_string();
    assert_eq!(protocol.parse::<BrowserSessionSid>().expect("UUID"), id);
    assert_eq!(id.to_string(), "[redacted]");
    assert!(!format!("{id:?}").contains(&protocol));
    let digest = browser_session_sid_digest(id);
    assert_eq!(digest.to_string(), "[redacted]");
    assert!(!format!("{digest:?}").contains(&protocol));
}

#[test]
fn digest_is_domain_separated_and_stable() {
    let id = BrowserSessionSid::from_uuid(Uuid::from_u128(1));
    let expected = [
        0x31, 0x4e, 0x24, 0x26, 0xd4, 0x0a, 0x7d, 0x68, 0x37, 0xa6, 0x57, 0x45, 0x9f, 0xde, 0x34,
        0x4e, 0x92, 0x37, 0x5e, 0xe2, 0x0a, 0x30, 0x4d, 0x0a, 0x63, 0xea, 0x56, 0x38, 0xbc, 0xc8,
        0xca, 0x1d,
    ];
    assert_eq!(browser_session_sid_digest(id).as_bytes(), expected);
    assert_ne!(
        browser_session_sid_digest(id),
        browser_session_sid_digest(BrowserSessionSid::from_uuid(Uuid::from_u128(2)))
    );
}

#[test]
fn identity_binding_digest_is_stable_and_length_delimited() {
    let identity = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(1)),
        "https://issuer.example",
        "subject",
        serde_json::Value::Null,
        RequestId::from_uuid(Uuid::from_u128(2)),
    );
    let expected = [
        0xa7, 0x45, 0x55, 0x1d, 0xf9, 0xfe, 0xc2, 0x16, 0x0f, 0x16, 0x54, 0x80, 0x4e, 0xa6, 0x97,
        0x5e, 0x2a, 0xa6, 0x4a, 0xca, 0xa2, 0x91, 0x7d, 0x51, 0x00, 0x98, 0x76, 0x4d, 0x88, 0x12,
        0xb3, 0x17,
    ];
    let digest = browser_session_identity_binding_digest(&identity);
    assert_eq!(digest.as_bytes(), expected);
    assert_eq!(digest.to_string(), "[redacted]");
    assert!(!format!("{digest:?}").contains("issuer.example"));

    let left = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(3)),
        "ab",
        "c",
        serde_json::Value::Null,
        RequestId::from_uuid(Uuid::from_u128(4)),
    );
    let right = AuthenticatedIdentity::new(
        UserId::from_uuid(Uuid::from_u128(3)),
        "a",
        "bc",
        serde_json::Value::Null,
        RequestId::from_uuid(Uuid::from_u128(4)),
    );
    assert_ne!(
        browser_session_identity_binding_digest(&left),
        browser_session_identity_binding_digest(&right)
    );
}

#[test]
fn metadata_enforces_bounded_lifetime_and_revocation() {
    let issued = OffsetDateTime::UNIX_EPOCH;
    let expires = issued + Duration::seconds(DEFAULT_BROWSER_SESSION_TTL_SECONDS);
    let metadata = BrowserSessionMetadata::new(
        BrowserSessionId::new(),
        UserId::new(),
        issued,
        expires,
        None,
    )
    .expect("valid metadata");
    assert!(!metadata.is_active_at(issued - Duration::seconds(1)));
    assert!(metadata.is_active_at(issued));
    assert!(metadata.is_active_at(issued + Duration::hours(1)));
    assert!(!metadata.is_active_at(expires));
    assert!(
        BrowserSessionMetadata::new(
            metadata.id(),
            metadata.user_id(),
            issued,
            issued + Duration::seconds(MAX_BROWSER_SESSION_TTL_SECONDS),
            None,
        )
        .is_some()
    );
    assert!(
        BrowserSessionMetadata::new(
            metadata.id(),
            metadata.user_id(),
            issued,
            issued + Duration::seconds(MAX_BROWSER_SESSION_TTL_SECONDS + 1),
            None,
        )
        .is_none()
    );
    assert!(
        BrowserSessionMetadata::new(metadata.id(), metadata.user_id(), issued, issued, None,)
            .is_none()
    );
    assert!(
        BrowserSessionMetadata::new(
            metadata.id(),
            metadata.user_id(),
            issued,
            expires,
            Some(issued - Duration::seconds(1)),
        )
        .is_none()
    );
    let revoked = BrowserSessionMetadata::new(
        metadata.id(),
        metadata.user_id(),
        issued,
        expires,
        Some(issued),
    )
    .expect("valid revoked metadata");
    assert!(!revoked.is_active_at(issued));
}
