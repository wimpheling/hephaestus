use super::*;
use crate::timestamp::parse_rfc3339;
use http::{HeaderMap, HeaderValue};
use registry_domain::Sha256Digest;
use time::OffsetDateTime;

const CALLBACK_TOKEN: &str = "0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG";
const EVENT_ID: &str = "a8098c1a-f86e-11da-bd1a-00112444be1e";
const EVENT_TIME: &str = "2026-08-04T12:00:00Z";
const DIGEST: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_static("Bearer 0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG"),
    );
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("ce-specversion", HeaderValue::from_static("1.0"));
    headers.insert("ce-id", HeaderValue::from_static(EVENT_ID));
    headers.insert("ce-source", HeaderValue::from_static("zotregistry.dev"));
    headers.insert(
        "ce-type",
        HeaderValue::from_static("zotregistry.image.updated"),
    );
    headers.insert("ce-time", HeaderValue::from_static(EVENT_TIME));
    headers
}

fn image_updated_body() -> String {
    format!(
        r#"{{"name":"platform/images/rust-ubuntu","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json","manifest":"{{}}","actor":{{"name":"worker"}},"request":{{"addr":"10.0.0.2:1234","method":"PUT","useragent":"skopeo"}}}}"#
    )
}

fn parse(body: &str) -> Result<NotificationObservation, NotificationError> {
    parse_notification(
        &headers(),
        body.as_bytes(),
        &CallbackCredential::parse(CALLBACK_TOKEN).expect("credential"),
        received_at(),
    )
}

fn received_at() -> OffsetDateTime {
    parse_rfc3339("2026-08-04T12:00:01Z").expect("fixed timestamp")
}

#[test]
fn parses_zot_image_updated_without_retaining_sensitive_or_raw_metadata() {
    let body = image_updated_body();
    let observation = parse(&body).expect("valid Zot event");

    assert_eq!(observation.action(), NotificationAction::Push);
    assert_eq!(observation.event_type(), ZotEventType::ImageUpdated);
    assert_eq!(
        observation.repository().as_str(),
        "platform/images/rust-ubuntu"
    );
    assert!(observation.repository().known_namespace().is_some());
    assert_eq!(observation.reference(), Some("latest"));
    assert_eq!(observation.digest().map(Sha256Digest::as_str), Some(DIGEST));
    assert_eq!(observation.body_size(), body.len());
    assert_eq!(observation.idempotency_key().as_str().len(), 64);
    assert_eq!(observation.payload_sha256().as_bytes().len(), 32);
    let debug = format!("{observation:?}");
    assert!(!debug.contains(CALLBACK_TOKEN));
    assert!(!debug.contains("10.0.0.2"));
    assert!(!debug.contains("skopeo"));
}

#[test]
fn duplicate_delivery_derives_the_same_key_and_body_hash() {
    let body = image_updated_body();
    let first = parse(&body).expect("first delivery");
    let second = parse(&body).expect("duplicate delivery");

    assert_eq!(first.idempotency_key(), second.idempotency_key());
    assert_eq!(first.payload_sha256(), second.payload_sha256());
}

#[test]
fn rejects_forged_callback_without_disclosing_the_configured_credential() {
    let mut forged = headers();
    forged.insert(
        "authorization",
        HeaderValue::from_static("Bearer AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
    );
    let credential = CallbackCredential::parse(CALLBACK_TOKEN).expect("credential");
    let error = parse_notification(
        &forged,
        image_updated_body().as_bytes(),
        &credential,
        received_at(),
    )
    .expect_err("forged callback must fail");

    assert_eq!(error, NotificationError::Unauthorized);
    assert!(!format!("{credential:?} {error:?}").contains(CALLBACK_TOKEN));
}

#[test]
fn accepts_unknown_but_canonical_paths_for_safe_reconciliation_diagnostics() {
    let body = image_updated_body().replace(
        "platform/images/rust-ubuntu",
        "projects/unknown/repository-images/orphan",
    );
    let observation = parse(&body).expect("bounded unknown namespace is observable");

    assert!(observation.repository().known_namespace().is_none());
    assert_eq!(
        observation.repository().as_str(),
        "projects/unknown/repository-images/orphan"
    );
}

#[test]
fn rejects_malformed_or_incomplete_zot_event_data() {
    let body = format!(
        r#"{{"name":"platform/builders/rust-ubuntu","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json"}}"#
    );
    assert_eq!(parse(&body), Err(NotificationError::InvalidEventFields));
}

#[test]
fn rejects_wrong_source_stale_time_and_duplicate_headers() {
    let credential = CallbackCredential::parse(CALLBACK_TOKEN).expect("credential");
    let mut wrong_source = headers();
    wrong_source.insert("ce-source", HeaderValue::from_static("attacker.invalid"));
    assert_eq!(
        parse_notification(
            &wrong_source,
            image_updated_body().as_bytes(),
            &credential,
            received_at(),
        ),
        Err(NotificationError::InvalidSource)
    );

    let mut stale = headers();
    stale.insert("ce-time", HeaderValue::from_static("2026-07-01T12:00:00Z"));
    assert_eq!(
        parse_notification(
            &stale,
            image_updated_body().as_bytes(),
            &credential,
            received_at(),
        ),
        Err(NotificationError::TimestampOutsideAcceptanceWindow)
    );

    let mut duplicate = headers();
    duplicate.append(
        "ce-type",
        HeaderValue::from_static("zotregistry.image.updated"),
    );
    assert_eq!(
        parse_notification(
            &duplicate,
            image_updated_body().as_bytes(),
            &credential,
            received_at(),
        ),
        Err(NotificationError::DuplicateHeader)
    );
}

#[test]
fn validates_all_documented_zot_event_shapes() {
    let credential = CallbackCredential::parse(CALLBACK_TOKEN).expect("credential");

    let repository_created = r#"{"name":"platform/builders/ubuntu-native"}"#;
    let mut created_headers = headers();
    created_headers.insert(
        "ce-type",
        HeaderValue::from_static("zotregistry.repository.created"),
    );
    assert_eq!(
        parse_notification(
            &created_headers,
            repository_created.as_bytes(),
            &credential,
            received_at(),
        )
        .expect("repository-created event")
        .action(),
        NotificationAction::Push
    );

    let deleted = format!(
        r#"{{"name":"platform/builders/ubuntu-native","reference":"latest","digest":"{DIGEST}","mediaType":"application/vnd.oci.image.manifest.v1+json"}}"#
    );
    let mut deleted_headers = headers();
    deleted_headers.insert(
        "ce-type",
        HeaderValue::from_static("zotregistry.image.deleted"),
    );
    assert_eq!(
        parse_notification(
            &deleted_headers,
            deleted.as_bytes(),
            &credential,
            received_at(),
        )
        .expect("image-deleted event")
        .action(),
        NotificationAction::Delete
    );

    let mut lint_headers = headers();
    lint_headers.insert(
        "ce-type",
        HeaderValue::from_static("zotregistry.image.lint_failed"),
    );
    assert_eq!(
        parse_notification(
            &lint_headers,
            image_updated_body().as_bytes(),
            &credential,
            received_at(),
        )
        .expect("image-lint-failed event")
        .event_type(),
        ZotEventType::ImageLintFailed
    );
}
