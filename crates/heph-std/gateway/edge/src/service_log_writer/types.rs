use std::time::Duration;

use crate::{GatewayServiceLogStoreError, ServiceLogLoss};

const MAX_APPEND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(60);
/// Default timeout for one durable append.
pub const DEFAULT_SERVICE_LOG_APPEND_TIMEOUT: Duration = Duration::from_secs(2);
/// Default initial delay between unavailable appends.
pub const DEFAULT_SERVICE_LOG_RETRY_INTERVAL: Duration = Duration::from_millis(250);
/// Default maximum delay between unavailable appends.
pub const DEFAULT_SERVICE_LOG_MAX_RETRY_INTERVAL: Duration = Duration::from_secs(5);
/// Default deadline for the final worker log flush.
pub const DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Retry and database wait bounds for one parent-owned log writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceLogWriterPolicy {
    /// Maximum time allowed for one durable append.
    pub append_timeout: Duration,
    /// Delay before the first retry after an unavailable append.
    pub retry_interval: Duration,
    /// Maximum delay between unavailable append attempts.
    pub max_retry_interval: Duration,
}

impl ServiceLogWriterPolicy {
    /// Creates a bounded writer policy.
    #[must_use]
    pub const fn new(
        append_timeout: Duration,
        retry_interval: Duration,
        max_retry_interval: Duration,
    ) -> Self {
        Self {
            append_timeout,
            retry_interval,
            max_retry_interval,
        }
    }

    pub(super) fn validate(self) -> Result<(), ServiceLogWriterError> {
        if self.append_timeout.is_zero()
            || self.append_timeout > MAX_APPEND_TIMEOUT
            || self.retry_interval.is_zero()
            || self.retry_interval > self.max_retry_interval
            || self.max_retry_interval > MAX_RETRY_INTERVAL
        {
            return Err(ServiceLogWriterError::InvalidPolicy);
        }
        Ok(())
    }
}

impl Default for ServiceLogWriterPolicy {
    fn default() -> Self {
        Self::new(
            DEFAULT_SERVICE_LOG_APPEND_TIMEOUT,
            DEFAULT_SERVICE_LOG_RETRY_INTERVAL,
            DEFAULT_SERVICE_LOG_MAX_RETRY_INTERVAL,
        )
    }
}

/// Redacted construction and append failures for a durable log writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServiceLogWriterError {
    /// A policy, lease, or owner identity was malformed.
    #[error("invalid service log writer configuration")]
    InvalidPolicy,
}

/// Progress from one parent-owned writer poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLogWriterPoll {
    /// No records or new loss counters were ready; the parent should pace the
    /// next poll with its own timer.
    Idle,
    /// The append was acknowledged and the writer can be polled again.
    Appended {
        /// Chunks newly retained by durable storage.
        accepted_chunks: u32,
        /// Chunks already retained with identical content.
        duplicate_chunks: u32,
        /// Chunks accounted as durable storage loss.
        storage_dropped_chunks: u32,
    },
    /// Durable capacity accounted for this batch; later batches remain eligible.
    Capacity {
        /// Chunks accounted as capacity loss.
        discarded_chunks: usize,
        /// Payload bytes accounted as capacity loss.
        discarded_bytes: usize,
    },
    /// Storage was unavailable; the same batch remains retained until retry.
    RetryScheduled {
        /// Delay before the next append attempt.
        delay: Duration,
    },
    /// The writer has stopped after a terminal append result or shutdown.
    Terminated {
        /// The durable reason, when storage rejected the batch.
        error: Option<GatewayServiceLogStoreError>,
    },
}

/// Result of a bounded final flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceLogWriterFlush {
    /// Whether all known records and loss counters were durably acknowledged.
    pub complete: bool,
    /// Known unacknowledged or terminally discarded records at the deadline.
    pub unflushed_chunks: usize,
    /// Known unacknowledged or terminally discarded payload bytes at the deadline.
    pub unflushed_bytes: usize,
    /// Loss counters not yet acknowledged by durable storage.
    pub unflushed_loss: ServiceLogLoss,
    /// Terminal reason, when the writer can no longer submit its lease.
    pub terminal_error: Option<GatewayServiceLogStoreError>,
}
