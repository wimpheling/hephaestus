//! Bounded, opt-in application log capture for one service instance.

use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
};

use time::OffsetDateTime;
use vm_trait::LogStream;

pub use gateway_domain::{
    GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome, GatewayServiceLogMaintenance,
    GatewayServiceLogMaintenanceError, GatewayServiceLogMaintenancePolicy,
    GatewayServiceLogMaintenanceProjectPage, GatewayServiceLogMaintenanceProjectPageResult,
    GatewayServiceLogMaintenanceProjects, GatewayServiceLogMaintenanceReport,
    GatewayServiceLogProjectMetadata, GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata,
    GatewayServiceLogReadPage, GatewayServiceLogReadRecord, GatewayServiceLogReadRequest,
    GatewayServiceLogReadScope, GatewayServiceLogStore, GatewayServiceLogStoreError,
    MAX_SERVICE_LOG_CHUNK_BYTES, MAX_SERVICE_LOG_INSTANCE_BYTES, MAX_SERVICE_LOG_INSTANCE_CHUNKS,
    MAX_SERVICE_LOG_MAINTENANCE_CHUNKS, MAX_SERVICE_LOG_MAINTENANCE_EPOCHS,
    MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE, MAX_SERVICE_LOG_PROJECT_BYTES,
    MAX_SERVICE_LOG_PROJECT_CHUNKS, MAX_SERVICE_LOG_PROJECT_EPOCHS, MAX_SERVICE_LOG_QUEUE_BYTES,
    MAX_SERVICE_LOG_QUEUE_CHUNKS, MAX_SERVICE_LOG_READ_PAGE_BYTES,
    MAX_SERVICE_LOG_READ_PAGE_RECORDS, ServiceLogBufferSnapshot, ServiceLogLoss, ServiceLogRecord,
};

#[derive(Default)]
struct ServiceLogBuffer {
    records: VecDeque<ServiceLogRecord>,
    queued_bytes: usize,
    next_sequence: u64,
    loss: ServiceLogLoss,
}

impl ServiceLogBuffer {
    fn record(&mut self, stream: LogStream, observed_at: OffsetDateTime, bytes: &[u8]) {
        let byte_count = bytes.len();
        let Some(sequence) = self
            .next_sequence
            .checked_add(1)
            .map(|_| self.next_sequence)
        else {
            self.loss.sequence_exhausted_chunks =
                self.loss.sequence_exhausted_chunks.saturating_add(1);
            self.loss.sequence_exhausted_bytes = self
                .loss
                .sequence_exhausted_bytes
                .saturating_add(u64::try_from(byte_count).unwrap_or(u64::MAX));
            return;
        };
        self.next_sequence += 1;
        if byte_count > MAX_SERVICE_LOG_CHUNK_BYTES {
            self.loss.oversized_chunks = self.loss.oversized_chunks.saturating_add(1);
            self.loss.oversized_bytes = self
                .loss
                .oversized_bytes
                .saturating_add(u64::try_from(byte_count).unwrap_or(u64::MAX));
            return;
        }
        if self.records.len() >= MAX_SERVICE_LOG_QUEUE_CHUNKS
            || self
                .queued_bytes
                .checked_add(byte_count)
                .is_none_or(|total| total > MAX_SERVICE_LOG_QUEUE_BYTES)
        {
            self.loss.queue_full_chunks = self.loss.queue_full_chunks.saturating_add(1);
            self.loss.queue_full_bytes = self
                .loss
                .queue_full_bytes
                .saturating_add(u64::try_from(byte_count).unwrap_or(u64::MAX));
            return;
        }
        self.queued_bytes += byte_count;
        self.records.push_back(ServiceLogRecord {
            sequence,
            stream,
            observed_at,
            bytes: bytes.to_vec(),
        });
    }

    fn snapshot(&self) -> ServiceLogBufferSnapshot {
        ServiceLogBufferSnapshot {
            queued_chunks: self.records.len(),
            queued_bytes: self.queued_bytes,
            next_sequence: self.next_sequence,
            loss: self.loss,
        }
    }

    fn drain(&mut self, max_chunks: usize, max_bytes: usize) -> Vec<ServiceLogRecord> {
        let mut drained = Vec::new();
        let mut bytes = 0_usize;
        while drained.len() < max_chunks {
            let Some(record) = self.records.front() else {
                break;
            };
            let record_bytes = record.bytes.len();
            if bytes
                .checked_add(record_bytes)
                .is_none_or(|total| total > max_bytes)
            {
                break;
            }
            let record = self.records.pop_front().expect("front record exists");
            bytes += record_bytes;
            self.queued_bytes -= record_bytes;
            drained.push(record);
        }
        drained
    }
}

/// Cloneable handle for the worker's bounded application log queue.
#[derive(Clone)]
pub struct ServiceLogBufferHandle {
    inner: Arc<Mutex<ServiceLogBuffer>>,
    busy_chunks: Arc<AtomicU64>,
    busy_bytes: Arc<AtomicU64>,
    provider_lagged_events: Arc<AtomicU64>,
}

fn saturating_add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

impl fmt::Debug for ServiceLogBufferHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceLogBufferHandle")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl ServiceLogBufferHandle {
    /// Creates an empty bounded queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ServiceLogBuffer::default())),
            busy_chunks: Arc::new(AtomicU64::new(0)),
            busy_bytes: Arc::new(AtomicU64::new(0)),
            provider_lagged_events: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Records one event without awaiting storage or blocking on a writer.
    pub fn try_record(&self, stream: LogStream, observed_at: OffsetDateTime, bytes: &[u8]) {
        if !matches!(stream, LogStream::Stdout | LogStream::Stderr) {
            return;
        }
        let byte_count = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        match self.inner.try_lock() {
            Ok(mut buffer) => buffer.record(stream, observed_at, bytes),
            Err(TryLockError::Poisoned(_) | TryLockError::WouldBlock) => {
                saturating_add(&self.busy_chunks, 1);
                saturating_add(&self.busy_bytes, byte_count);
            }
        }
    }

    /// Returns bounded queue and loss state for a later writer or reader.
    ///
    /// # Panics
    ///
    /// Panics if another thread previously poisoned the queue mutex.
    #[must_use]
    pub fn snapshot(&self) -> ServiceLogBufferSnapshot {
        let mut snapshot = self
            .inner
            .lock()
            .expect("service log buffer lock")
            .snapshot();
        snapshot.loss.busy_chunks = self.busy_chunks.load(Ordering::Relaxed);
        snapshot.loss.busy_bytes = self.busy_bytes.load(Ordering::Relaxed);
        snapshot.loss.provider_lagged_events = self.provider_lagged_events.load(Ordering::Relaxed);
        snapshot
    }

    /// Removes at most the requested number of chunks and bytes.
    ///
    /// # Panics
    ///
    /// Panics if another thread previously poisoned the queue mutex.
    #[must_use]
    pub fn drain(&self, max_chunks: usize, max_bytes: usize) -> Vec<ServiceLogRecord> {
        let max_chunks = max_chunks.min(MAX_SERVICE_LOG_QUEUE_CHUNKS);
        let max_bytes = max_bytes.min(MAX_SERVICE_LOG_QUEUE_BYTES);
        self.inner
            .lock()
            .expect("service log buffer lock")
            .drain(max_chunks, max_bytes)
    }

    /// Records provider-side event loss without blocking on the queue.
    pub fn record_provider_lag(&self, skipped: u64) {
        saturating_add(&self.provider_lagged_events, skipped);
    }
}

impl Default for ServiceLogBufferHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod append_tests {
    use super::*;

    fn record(sequence: u64) -> ServiceLogRecord {
        ServiceLogRecord {
            sequence,
            stream: LogStream::Stdout,
            observed_at: OffsetDateTime::UNIX_EPOCH,
            bytes: b"line".to_vec(),
        }
    }

    #[test]
    fn append_batch_requires_strictly_ordered_bounded_records() {
        assert!(
            GatewayServiceLogAppendBatch::new(
                vec![record(0), record(1)],
                ServiceLogLoss::default()
            )
            .is_ok()
        );
        assert!(
            GatewayServiceLogAppendBatch::new(
                vec![record(1), record(1)],
                ServiceLogLoss::default()
            )
            .is_err()
        );
        assert!(
            GatewayServiceLogAppendBatch::new(
                vec![record(2), record(1)],
                ServiceLogLoss::default()
            )
            .is_err()
        );
        assert!(
            GatewayServiceLogAppendBatch::new(
                vec![ServiceLogRecord {
                    sequence: 0,
                    stream: LogStream::Stdout,
                    observed_at: OffsetDateTime::UNIX_EPOCH,
                    bytes: vec![0; MAX_SERVICE_LOG_CHUNK_BYTES + 1],
                }],
                ServiceLogLoss::default()
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod tests {
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
}

#[cfg(test)]
mod read_contract_tests {
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
            GatewayServiceLogReadRequest::new(
                read_scope,
                MAX_SERVICE_LOG_READ_PAGE_RECORDS + 1,
                None
            )
            .is_err()
        );
        assert!(GatewayServiceLogReadRequest::new(scope(20), 10, Some(cursor)).is_err());
        assert!(GatewayServiceLogReadCursor::new(read_scope, u64::MAX).is_err());
        let mut assembled = GatewayServiceLogReadRequest::new(read_scope, 10, None)
            .expect("valid assembled request");
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
}
