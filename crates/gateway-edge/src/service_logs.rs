//! Bounded, opt-in application log capture for one service instance.

use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

use crate::{GatewayServiceInstanceLease, GatewayServiceOwner};

/// Maximum bytes accepted from one provider log event.
pub const MAX_SERVICE_LOG_CHUNK_BYTES: usize = 64 * 1024;
/// Maximum number of chunks retained before a database writer drains them.
pub const MAX_SERVICE_LOG_QUEUE_CHUNKS: usize = 64;
/// Maximum bytes retained before a database writer drains the queue.
pub const MAX_SERVICE_LOG_QUEUE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum retained log bytes for one instance across all fencing epochs.
pub const MAX_SERVICE_LOG_INSTANCE_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum retained log chunks for one instance across all fencing epochs.
pub const MAX_SERVICE_LOG_INSTANCE_CHUNKS: u64 = 4096;
/// Maximum retained log bytes for one project.
pub const MAX_SERVICE_LOG_PROJECT_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum retained log chunks for one project.
pub const MAX_SERVICE_LOG_PROJECT_CHUNKS: u64 = 65_536;
/// Maximum chunks one bounded retention transaction may inspect.
pub const MAX_SERVICE_LOG_MAINTENANCE_CHUNKS: usize = 256;
/// Maximum epoch metadata rows one bounded retention transaction may inspect.
pub const MAX_SERVICE_LOG_MAINTENANCE_EPOCHS: usize = 32;
/// Maximum retained fencing epochs represented in one project's log metadata.
pub const MAX_SERVICE_LOG_PROJECT_EPOCHS: u32 = 128;

/// One bounded application log chunk observed from the guest.
#[derive(Clone)]
pub struct ServiceLogRecord {
    /// Monotonically increasing sequence assigned to each event reaching the
    /// queue lock.
    pub sequence: u64,
    /// Guest output stream.
    pub stream: LogStream,
    /// Host time at which the provider event was observed.
    pub observed_at: OffsetDateTime,
    /// Uninterpreted application bytes.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for ServiceLogRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceLogRecord")
            .field("sequence", &self.sequence)
            .field("stream", &self.stream)
            .field("observed_at", &self.observed_at)
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

/// Loss counters accompanying a bounded service log queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServiceLogLoss {
    /// Number of chunks rejected because they exceeded the per-event limit.
    pub oversized_chunks: u64,
    /// Bytes rejected from oversized chunks.
    pub oversized_bytes: u64,
    /// Number of chunks rejected because the queue was full.
    pub queue_full_chunks: u64,
    /// Bytes rejected because the queue was full.
    pub queue_full_bytes: u64,
    /// Number of chunks rejected while the bounded queue lock was busy.
    pub busy_chunks: u64,
    /// Bytes rejected while the bounded queue lock was busy.
    pub busy_bytes: u64,
    /// Provider events skipped before this consumer could receive them.
    pub provider_lagged_events: u64,
    /// Number of chunks rejected after sequence space was exhausted.
    pub sequence_exhausted_chunks: u64,
    /// Bytes rejected after sequence space was exhausted.
    pub sequence_exhausted_bytes: u64,
}

impl ServiceLogLoss {
    /// Returns the total number of chunks that were not retained.
    #[must_use]
    pub const fn total_chunks(self) -> u64 {
        self.oversized_chunks
            .saturating_add(self.queue_full_chunks)
            .saturating_add(self.busy_chunks)
            .saturating_add(self.sequence_exhausted_chunks)
    }

    /// Returns known bytes that were not retained; provider-lag bytes are
    /// unavailable because those events were never received.
    #[must_use]
    pub const fn total_bytes(self) -> u64 {
        self.oversized_bytes
            .saturating_add(self.queue_full_bytes)
            .saturating_add(self.busy_bytes)
            .saturating_add(self.sequence_exhausted_bytes)
    }
}

/// Bounded queue state visible to a later durable writer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServiceLogBufferSnapshot {
    /// Number of queued chunks awaiting a writer.
    pub queued_chunks: usize,
    /// Bytes currently queued.
    pub queued_bytes: usize,
    /// Sequence that will be assigned to the next observed event.
    pub next_sequence: u64,
    /// Explicitly accounted dropped chunks and bytes. Provider lag counts are
    /// separate because skipped event kinds and byte lengths are unknown.
    pub loss: ServiceLogLoss,
}

/// A bounded batch handed from one worker-owned queue to durable storage.
#[derive(Debug, Clone, Default)]
pub struct GatewayServiceLogAppendBatch {
    /// Queued application chunks, in the order observed by the worker.
    pub records: Vec<ServiceLogRecord>,
    /// Producer-side losses observed while collecting this batch.
    pub loss: ServiceLogLoss,
}

impl GatewayServiceLogAppendBatch {
    /// Constructs a batch and rejects values that cannot be stored safely.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLogStoreError::InvalidArgument`] when a batch
    /// exceeds the queue bound, contains an oversized chunk, or is not in
    /// strictly increasing sequence order.
    pub fn new(
        records: Vec<ServiceLogRecord>,
        loss: ServiceLogLoss,
    ) -> Result<Self, GatewayServiceLogStoreError> {
        let batch = Self { records, loss };
        batch.validate()?;
        Ok(batch)
    }

    /// Validates an already-owned batch without copying its records.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLogStoreError::InvalidArgument`] when a batch
    /// exceeds the queue bound, contains an oversized chunk, or is not in
    /// strictly increasing sequence order.
    pub fn validate(&self) -> Result<(), GatewayServiceLogStoreError> {
        let records = &self.records;
        if records.len() > MAX_SERVICE_LOG_QUEUE_CHUNKS
            || records.iter().any(|record| {
                !matches!(record.stream, LogStream::Stdout | LogStream::Stderr)
                    || record.bytes.len() > MAX_SERVICE_LOG_CHUNK_BYTES
                    || record.sequence > i64::MAX as u64
            })
        {
            return Err(GatewayServiceLogStoreError::InvalidArgument);
        }
        if records
            .windows(2)
            .any(|window| window[0].sequence >= window[1].sequence)
        {
            return Err(GatewayServiceLogStoreError::InvalidArgument);
        }
        Ok(())
    }
}

/// Durable counters returned after one append transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatewayServiceLogAppendOutcome {
    /// Chunks inserted during this call.
    pub accepted_chunks: u32,
    /// Retained rows already present with identical content.
    pub duplicate_chunks: u32,
    /// New chunks rejected by durable capacity bounds.
    pub storage_dropped_chunks: u32,
    /// Highest worker sequence durably acknowledged, including dropped or
    /// previously evicted sequences.
    pub acknowledged_through: Option<u64>,
    /// Current retained bytes for the exact instance.
    pub retained_instance_bytes: u64,
    /// Current retained chunks for the exact instance.
    pub retained_instance_chunks: u64,
}

/// Safe failures for a worker log append. Raw SQL/provider details stay out of
/// the caller-visible contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceLogStoreError {
    /// The caller supplied malformed identity, owner, sequence, or payload.
    #[error("invalid gateway service log append argument")]
    InvalidArgument,
    /// The durable instance is no longer owned by this worker/fence.
    #[error("gateway service log append lease is stale")]
    StaleLease,
    /// The immutable revision is not opted into application log capture.
    #[error("gateway service log capture is disabled")]
    Disabled,
    /// A durable identity or retained payload conflicts with this append.
    #[error("gateway service log append conflicts with durable state")]
    Conflict,
    /// The bounded durable metadata or quota could not accept a new epoch.
    /// This is terminal for the batch: the caller must discard it after
    /// recording the reported loss and must not retry it under this epoch.
    #[error("gateway service log append capacity is exhausted")]
    Capacity,
    /// Storage could not complete the bounded transaction.
    #[error("gateway service log storage is unavailable")]
    Unavailable,
}

/// Worker-owned durable append port. Implementations must derive project and
/// revision policy from the durable identity rather than caller assertions.
#[async_trait]
pub trait GatewayServiceLogStore: Send + Sync {
    /// Appends one bounded queue batch for the exact current lease.
    async fn append_batch(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        batch: GatewayServiceLogAppendBatch,
    ) -> Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>;
}

/// Bounded work policy for one worker-owned retention transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenancePolicy {
    /// Maximum payload chunks deleted in one transaction. TTL and compensated
    /// pressure scans may inspect more candidates, but deletion is bounded.
    pub max_chunks: usize,
    /// Maximum empty epoch rows deleted in one transaction. Flag updates may
    /// lock the bounded project epoch set in addition to these deletions.
    pub max_epochs: usize,
}

impl GatewayServiceLogMaintenancePolicy {
    /// Creates a bounded retention policy.
    #[must_use]
    pub const fn new(max_chunks: usize, max_epochs: usize) -> Self {
        Self {
            max_chunks,
            max_epochs,
        }
    }

    /// Validates the policy against platform work limits.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.max_chunks > 0
            && self.max_chunks <= MAX_SERVICE_LOG_MAINTENANCE_CHUNKS
            && self.max_epochs > 0
            && self.max_epochs <= MAX_SERVICE_LOG_MAINTENANCE_EPOCHS
    }
}

impl Default for GatewayServiceLogMaintenancePolicy {
    fn default() -> Self {
        Self::new(
            MAX_SERVICE_LOG_MAINTENANCE_CHUNKS,
            MAX_SERVICE_LOG_MAINTENANCE_EPOCHS,
        )
    }
}

/// Redacted result of one bounded retention transaction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenanceReport {
    /// Chunks removed because their server retention time elapsed.
    pub expired_chunks: usize,
    /// Bytes removed because their server retention time elapsed.
    pub expired_bytes: usize,
    /// Chunks removed under instance or project pressure.
    pub evicted_chunks: usize,
    /// Bytes removed under instance or project pressure.
    pub evicted_bytes: usize,
    /// Empty, permanently ineligible epoch metadata rows removed.
    pub metadata_epochs: usize,
    /// Whether another bounded transaction may have eligible work.
    pub has_more: bool,
}

/// Safe failures for a worker-owned retention transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceLogMaintenanceError {
    /// The project or bounded policy is malformed.
    #[error("invalid gateway service log maintenance argument")]
    InvalidArgument,
    /// `PostgreSQL` could not complete the bounded transaction.
    #[error("gateway service log maintenance is unavailable")]
    Unavailable,
}

/// Worker-only bounded retention and metadata maintenance port.
#[async_trait]
pub trait GatewayServiceLogMaintenance: Send + Sync {
    /// Performs one bounded server-clock TTL, pressure, and metadata pass.
    async fn maintain_project(
        &self,
        project_id: Uuid,
        policy: GatewayServiceLogMaintenancePolicy,
    ) -> Result<GatewayServiceLogMaintenanceReport, GatewayServiceLogMaintenanceError>;
}

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
