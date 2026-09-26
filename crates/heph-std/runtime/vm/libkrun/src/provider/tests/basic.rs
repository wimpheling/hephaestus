// Scenario tests intentionally retain Arc and broker handles across awaits so
// concurrent lifecycle behavior remains observable by each task.
#![allow(clippy::significant_drop_tightening)]
use super::support::*;

#[test]
fn runtime_directory_is_private_and_collision_is_typed() {
    let temp = TempDir::new().unwrap();
    let runtime = create_runtime_dir(temp.path(), "vm").unwrap();
    assert_eq!(
        fs::metadata(runtime).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert!(matches!(
        create_runtime_dir(temp.path(), "vm"),
        Err(VmError::AlreadyExists(_))
    ));
}
#[test]
fn worker_errors_remain_typed() {
    let error = wire_to_vm_error(WireError {
        kind: WireErrorKind::Unavailable,
        code: "passt".to_owned(),
        message: "not installed".to_owned(),
    });
    assert!(matches!(
        error,
        VmError::Unavailable { resource, .. } if resource == "passt"
    ));
}
#[test]
fn private_http_response_preserves_one_bounded_mailbox_publication() {
    let response = private_http_response(PrivateHttpResponseMessage {
        status: 202,
        headers: Vec::new(),
        body: Vec::new(),
        mailbox_publication: Some(crate::protocol::PrivateMailboxPublicationMessage {
            slot: "recipe-events".to_owned(),
            method: "POST".to_owned(),
            route: "/recipes".to_owned(),
            headers: vec![("source".to_owned(), "gateway".to_owned())],
            content_type: Some("application/json".to_owned()),
            trace_context: Some("trace-42".to_owned()),
            body: b"event".to_vec(),
            deduplication_key: "telegram-update-42".to_owned(),
        }),
    })
    .expect("bounded response");
    let publication = response.mailbox_publication.expect("publication");
    assert_eq!(publication.slot, "recipe-events");
    assert_eq!(publication.body, bytes::Bytes::from_static(b"event"));
    assert_eq!(publication.deduplication_key, "telegram-update-42");
}
#[test]
fn private_http_response_rejects_an_invalid_mailbox_publication() {
    let response = private_http_response(PrivateHttpResponseMessage {
        status: 202,
        headers: Vec::new(),
        body: Vec::new(),
        mailbox_publication: Some(crate::protocol::PrivateMailboxPublicationMessage {
            slot: "recipe-events".to_owned(),
            method: "post".to_owned(),
            route: "/recipes".to_owned(),
            headers: Vec::new(),
            content_type: None,
            trace_context: None,
            body: Vec::new(),
            deduplication_key: "update-42".to_owned(),
        }),
    });
    assert!(matches!(response, Err(VmError::InvalidSpec { .. })));
}
