//! Worker-owned bounded service-log buffering and durable append contracts.

use std::fmt;

use async_trait::async_trait;
use time::OffsetDateTime;
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
