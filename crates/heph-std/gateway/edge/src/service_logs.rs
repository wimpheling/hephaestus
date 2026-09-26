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
#[path = "service_logs/append_tests.rs"]
mod append_tests;

#[cfg(test)]
#[path = "service_logs/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "service_logs/read_contract_tests.rs"]
mod read_contract_tests;
