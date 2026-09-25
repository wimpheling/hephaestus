use super::*;
use uuid::Uuid;

fn scope(offset: u128) -> GatewayServiceLogReadScope {
    GatewayServiceLogReadScope::new(
        Uuid::from_u128(offset + 1),
        Uuid::from_u128(offset + 2),
        Uuid::from_u128(offset + 3),
        Uuid::from_u128(offset + 4),
        7,
    )
    .expect("valid read scope")
}

#[test]
fn read_request_bounds_page_and_binds_cursor_to_all_scope_fields() {
    let read_scope = scope(10);
    let cursor = GatewayServiceLogReadCursor::new(read_scope, 42).expect("valid cursor");
    assert!(GatewayServiceLogReadRequest::new(read_scope, 1, Some(cursor)).is_ok());
    assert!(
        GatewayServiceLogReadRequest::new(read_scope, MAX_SERVICE_LOG_READ_PAGE_RECORDS, None)
            .is_ok()
    );
    assert!(GatewayServiceLogReadRequest::new(read_scope, 0, None).is_err());
    assert!(
        GatewayServiceLogReadRequest::new(read_scope, MAX_SERVICE_LOG_READ_PAGE_RECORDS + 1, None)
            .is_err()
    );
    assert!(GatewayServiceLogReadRequest::new(scope(20), 10, Some(cursor)).is_err());
    assert!(GatewayServiceLogReadCursor::new(read_scope, u64::MAX).is_err());
    let mut assembled =
        GatewayServiceLogReadRequest::new(read_scope, 10, None).expect("valid assembled request");
    assembled.limit = 0;
    assert!(assembled.validate().is_err());
}

#[test]
fn read_scope_rejects_nil_identity_and_nonpositive_fence() {
    let valid = scope(30);
    assert!(
        GatewayServiceLogReadScope::new(
            Uuid::nil(),
            valid.gateway_id,
            valid.revision_id,
            valid.instance_id,
            valid.fencing_token,
        )
        .is_err()
    );
    assert!(
        GatewayServiceLogReadScope::new(
            valid.project_id,
            valid.gateway_id,
            valid.revision_id,
            valid.instance_id,
            0,
        )
        .is_err()
    );
    assert!(
        GatewayServiceLogReadScope::new(
            valid.project_id,
            valid.gateway_id,
            valid.revision_id,
            valid.instance_id,
            -1,
        )
        .is_err()
    );
}

#[test]
fn read_record_debug_redacts_application_bytes() {
    let record = GatewayServiceLogReadRecord {
        sequence: 3,
        stream: LogStream::Stderr,
        observed_at: OffsetDateTime::UNIX_EPOCH,
        stored_at: OffsetDateTime::UNIX_EPOCH,
        bytes: b"application-secret-like-value".to_vec(),
    };
    let debug = format!("{record:?}");
    assert!(debug.contains("byte_len"));
    assert!(!debug.contains("application-secret-like-value"));
}

#[test]
fn empty_epoch_metadata_is_distinct_from_history_gap() {
    let metadata = GatewayServiceLogReadMetadata::default();
    assert!(!metadata.epoch_present);
    assert_eq!(metadata.earliest_retained_sequence, None);
    let page = GatewayServiceLogReadPage {
        records: Vec::new(),
        metadata,
        history_incomplete: false,
        next_after: None,
    };
    assert!(!page.history_incomplete);
}
