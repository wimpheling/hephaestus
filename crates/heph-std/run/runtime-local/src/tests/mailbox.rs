use crate::context::materialize_mailbox_event;
use crate::filesystem::runtime_mount_tag;
use crate::{CONTEXT_TAG_PREFIX, PREVIOUS_RELEASE_TAG_PREFIX, RELEASE_TAG_PREFIX};
use run_orchestrator::MailboxRuntimeEvent;
use runtime_types::RunId;
use sha2::{Digest, Sha256};
use std::fs;
use uuid::Uuid;

#[test]
fn mailbox_input_is_sealed_as_a_control_envelope_and_opaque_body() {
    let fixture = tempfile::tempdir().expect("fixture");
    let mailbox_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let body_id = Uuid::new_v4();
    let body = b"guest-only-mailbox-body".to_vec();
    materialize_mailbox_event(
        fixture.path(),
        &MailboxRuntimeEvent {
            mailbox_id,
            event_id,
            body_id,
            method: String::from("POST"),
            route: String::from("/event"),
            selected_headers: serde_json::json!({"x-kind": "proof"}),
            content_type: Some(String::from("application/octet-stream")),
            trace_context: None,
            received_at: time::OffsetDateTime::now_utc(),
            body: body.clone(),
            integrity_hash: Sha256::digest(&body).into(),
        },
    )
    .expect("materialize mailbox input");
    assert_eq!(
        fs::read(fixture.path().join("mailbox-body")).expect("read body"),
        body
    );
    let envelope: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.path().join("mailbox-event.json")).expect("read envelope"),
    )
    .expect("parse envelope");
    assert_eq!(envelope["mailbox_id"], mailbox_id.to_string());
    assert_eq!(envelope["event_id"], event_id.to_string());
    assert_eq!(envelope["body_id"], body_id.to_string());
    assert_eq!(envelope["body_path"], "/run/hephaestus/mailbox-body");
}

#[test]
fn runtime_mount_tags_fit_the_libkrun_limit() {
    let run_id = RunId::new();

    for prefix in [
        RELEASE_TAG_PREFIX,
        CONTEXT_TAG_PREFIX,
        PREVIOUS_RELEASE_TAG_PREFIX,
    ] {
        let tag = runtime_mount_tag(prefix, run_id);
        assert_eq!(tag.len(), 36);
        assert_eq!(tag, runtime_mount_tag(prefix, run_id));
    }
}
