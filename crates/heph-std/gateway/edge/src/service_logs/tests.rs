use super::*;
use uuid::Uuid;

#[test]
fn maintenance_project_page_validates_bounded_uuid_cursor() {
    assert!(GatewayServiceLogMaintenanceProjectPage::new(None, 1).is_ok());
    assert!(
        GatewayServiceLogMaintenanceProjectPage::new(
            None,
            MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE,
        )
        .is_ok()
    );
    assert!(GatewayServiceLogMaintenanceProjectPage::new(None, 0).is_err());
    assert!(
        GatewayServiceLogMaintenanceProjectPage::new(
            None,
            MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE + 1,
        )
        .is_err()
    );
    assert!(GatewayServiceLogMaintenanceProjectPage::new(Some(Uuid::nil()), 1).is_err());
}

#[test]
fn assigns_gaps_and_reports_oversized_events() {
    let buffer = ServiceLogBufferHandle::new();
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, &[1]);
    let oversized = vec![0; MAX_SERVICE_LOG_CHUNK_BYTES + 1];
    buffer.try_record(LogStream::Stderr, OffsetDateTime::UNIX_EPOCH, &oversized);
    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.next_sequence, 2);
    assert_eq!(snapshot.loss.oversized_chunks, 1);
    assert_eq!(
        snapshot.loss.oversized_bytes,
        (MAX_SERVICE_LOG_CHUNK_BYTES + 1) as u64
    );
    assert_eq!(buffer.drain(8, 8).first().expect("record").sequence, 0);
}

#[test]
fn bounds_queue_and_drain_without_losing_order() {
    let buffer = ServiceLogBufferHandle::new();
    for _ in 0..=MAX_SERVICE_LOG_QUEUE_CHUNKS {
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, &[7]);
    }
    assert_eq!(buffer.snapshot().loss.queue_full_chunks, 1);
    let records = buffer.drain(3, 3);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].sequence, 0);
    assert_eq!(records[2].sequence, 2);
    assert_eq!(
        buffer.snapshot().queued_chunks,
        MAX_SERVICE_LOG_QUEUE_CHUNKS - 3
    );
}

#[test]
fn reports_provider_lag_without_requiring_raw_bytes() {
    let buffer = ServiceLogBufferHandle::new();
    buffer.record_provider_lag(7);
    let snapshot = buffer.snapshot();
    assert_eq!(snapshot.loss.provider_lagged_events, 7);
    assert_eq!(snapshot.loss.total_chunks(), 0);
    assert_eq!(snapshot.loss.total_bytes(), 0);
}

#[test]
fn lock_contention_and_sequence_exhaustion_are_explicit_loss() {
    let buffer = ServiceLogBufferHandle::new();
    let guard = buffer.inner.try_lock().expect("queue lock");
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"secret");
    drop(guard);
    assert_eq!(buffer.snapshot().loss.busy_chunks, 1);
    assert!(!format!("{buffer:?}").contains("secret"));

    buffer.inner.lock().expect("queue lock").next_sequence = u64::MAX;
    buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"late");
    assert_eq!(buffer.snapshot().loss.sequence_exhausted_chunks, 1);
}
