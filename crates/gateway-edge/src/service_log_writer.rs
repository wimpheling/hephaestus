//! Parent-owned durable pumping for one service instance's application logs.
//!
//! The writer retains at most one 64-record/4 MiB batch. The append call gets
//! a bounded clone of that batch, so a transient cancellation temporarily uses
//! at most two such payload copies. The parent must settle the worker's event
//! collector before calling [`ServiceLogWriter::final_flush`], and must keep
//! lease supervision active until the flush result is handled.

use std::{fmt, sync::Arc, time::Duration};

use tokio::time::{self, Instant};

use crate::{
    GatewayServiceInstanceLease, GatewayServiceLogAppendBatch, GatewayServiceLogStore,
    GatewayServiceLogStoreError, GatewayServiceOwner, MAX_SERVICE_LOG_QUEUE_BYTES,
    MAX_SERVICE_LOG_QUEUE_CHUNKS, ServiceLogBufferHandle, ServiceLogBufferSnapshot, ServiceLogLoss,
};

const MAX_APPEND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(60);

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

    fn validate(self) -> Result<(), ServiceLogWriterError> {
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

/// Parent-owned durable writer for one immutable service lease.
pub struct ServiceLogWriter<O: ?Sized> {
    buffer: ServiceLogBufferHandle,
    store: Arc<O>,
    lease: GatewayServiceInstanceLease,
    owner: GatewayServiceOwner,
    policy: ServiceLogWriterPolicy,
    pending: Option<GatewayServiceLogAppendBatch>,
    flushed_loss: ServiceLogLoss,
    retry_at: Option<Instant>,
    retry_delay: Duration,
    shutdown_requested: bool,
    terminal: bool,
    terminal_error: Option<GatewayServiceLogStoreError>,
    terminal_discarded: (usize, usize),
}

impl<O: ?Sized> fmt::Debug for ServiceLogWriter<O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceLogWriter")
            .field("instance_id", &self.lease.identity.instance_id)
            .field("fencing_token", &self.lease.fencing_token)
            .field(
                "pending_chunks",
                &self.pending.as_ref().map_or(0, |batch| batch.records.len()),
            )
            .field(
                "pending_bytes",
                &self.pending.as_ref().map_or(0, |batch| {
                    batch
                        .records
                        .iter()
                        .map(|record| record.bytes.len())
                        .sum::<usize>()
                }),
            )
            .field("terminal", &self.terminal)
            .field("shutdown_requested", &self.shutdown_requested)
            .finish_non_exhaustive()
    }
}

impl<O> ServiceLogWriter<O>
where
    O: GatewayServiceLogStore + ?Sized,
{
    /// Binds a writer to one immutable lease and owner identity.
    ///
    /// The lease and owner are copied into the writer. A later ownership
    /// takeover cannot cause queued records to be submitted under its fence.
    ///
    /// # Errors
    ///
    /// Returns [`ServiceLogWriterError::InvalidPolicy`] for an invalid retry
    /// policy or an owner/lease identity mismatch.
    pub fn new(
        buffer: ServiceLogBufferHandle,
        store: Arc<O>,
        lease: GatewayServiceInstanceLease,
        owner: GatewayServiceOwner,
        policy: ServiceLogWriterPolicy,
    ) -> Result<Self, ServiceLogWriterError> {
        policy.validate()?;
        if lease.fencing_token <= 0 || lease.owner_uuid != owner.owner_uuid {
            return Err(ServiceLogWriterError::InvalidPolicy);
        }
        if lease.owner_host_id != owner.host_id {
            return Err(ServiceLogWriterError::InvalidPolicy);
        }
        if lease.identity.instance_id.is_nil()
            || lease.identity.gateway_id.is_nil()
            || lease.identity.revision_id.is_nil()
        {
            return Err(ServiceLogWriterError::InvalidPolicy);
        }
        owner
            .validate()
            .map_err(|_| ServiceLogWriterError::InvalidPolicy)?;
        Ok(Self {
            buffer,
            store,
            lease,
            owner,
            policy,
            pending: None,
            flushed_loss: ServiceLogLoss::default(),
            retry_at: None,
            retry_delay: policy.retry_interval,
            shutdown_requested: false,
            terminal: false,
            terminal_error: None,
            terminal_discarded: (0, 0),
        })
    }

    /// Returns the exact immutable identity bound to this writer.
    #[must_use]
    pub const fn lease(&self) -> &GatewayServiceInstanceLease {
        &self.lease
    }

    /// Returns a bounded queue snapshot without exposing payload bytes.
    #[must_use]
    pub fn snapshot(&self) -> ServiceLogBufferSnapshot {
        self.buffer.snapshot()
    }

    /// Polls one bounded append attempt.
    ///
    /// A retryable storage failure leaves the current batch owned by this
    /// writer. Dropping this future therefore cannot lose or rebind it.
    pub async fn poll(&mut self) -> ServiceLogWriterPoll {
        if self.terminal {
            return ServiceLogWriterPoll::Terminated {
                error: self.terminal_error,
            };
        }
        if let Some(retry_at) = self.retry_at {
            if retry_at > Instant::now() {
                time::sleep_until(retry_at).await;
            }
            self.retry_at = None;
        }
        if self.pending.is_none() {
            self.prepare_batch();
        }
        let Some(batch) = self.pending.as_ref() else {
            if self.shutdown_requested {
                self.terminal = true;
                return ServiceLogWriterPoll::Terminated { error: None };
            }
            return ServiceLogWriterPoll::Idle;
        };
        let batch_loss = batch.loss;
        let result = time::timeout(
            self.policy.append_timeout,
            self.store
                .append_batch(&self.lease, &self.owner, batch.clone()),
        )
        .await;
        match result {
            Ok(Ok(outcome)) => {
                self.pending = None;
                self.flushed_loss = batch_loss;
                self.retry_delay = self.policy.retry_interval;
                ServiceLogWriterPoll::Appended {
                    accepted_chunks: outcome.accepted_chunks,
                    duplicate_chunks: outcome.duplicate_chunks,
                    storage_dropped_chunks: outcome.storage_dropped_chunks,
                }
            }
            Ok(Err(GatewayServiceLogStoreError::Unavailable)) | Err(_) => {
                let delay = self.retry_delay;
                self.retry_at = Instant::now().checked_add(delay);
                self.retry_delay = self
                    .retry_delay
                    .checked_mul(2)
                    .unwrap_or(self.policy.max_retry_interval)
                    .min(self.policy.max_retry_interval);
                ServiceLogWriterPoll::RetryScheduled { delay }
            }
            Ok(Err(GatewayServiceLogStoreError::Capacity)) => {
                let discarded =
                    batch
                        .records
                        .iter()
                        .fold((0_usize, 0_usize), |(chunks, bytes), record| {
                            (
                                chunks.saturating_add(1),
                                bytes.saturating_add(record.bytes.len()),
                            )
                        });
                self.pending = None;
                let delay = self.retry_delay;
                self.retry_at = Instant::now().checked_add(delay);
                self.retry_delay = self
                    .retry_delay
                    .checked_mul(2)
                    .unwrap_or(self.policy.max_retry_interval)
                    .min(self.policy.max_retry_interval);
                ServiceLogWriterPoll::Capacity {
                    discarded_chunks: discarded.0,
                    discarded_bytes: discarded.1,
                }
            }
            Ok(Err(
                error @ (GatewayServiceLogStoreError::StaleLease
                | GatewayServiceLogStoreError::Disabled
                | GatewayServiceLogStoreError::InvalidArgument
                | GatewayServiceLogStoreError::Conflict),
            )) => {
                let discarded =
                    batch
                        .records
                        .iter()
                        .fold((0_usize, 0_usize), |(chunks, bytes), record| {
                            (
                                chunks.saturating_add(1),
                                bytes.saturating_add(record.bytes.len()),
                            )
                        });
                self.pending = None;
                self.terminal = true;
                self.terminal_error = Some(error);
                self.terminal_discarded = discarded;
                ServiceLogWriterPoll::Terminated { error: Some(error) }
            }
        }
    }

    /// Flushes known queue data until the deadline, retaining any unfinished
    /// batch for a later parent-owned recovery attempt.
    pub async fn final_flush(&mut self, deadline: Instant) -> ServiceLogWriterFlush {
        while !self.terminal && Instant::now() < deadline {
            let Ok(poll) = time::timeout_at(deadline, self.poll()).await else {
                break;
            };
            if matches!(
                poll,
                ServiceLogWriterPoll::Idle | ServiceLogWriterPoll::Terminated { .. }
            ) {
                break;
            }
        }
        let (unflushed_chunks, unflushed_bytes) = self.unflushed_counts();
        let unflushed_loss = loss_difference(self.buffer.snapshot().loss, self.flushed_loss);
        if self.pending.is_none()
            && unflushed_chunks == 0
            && unflushed_bytes == 0
            && unflushed_loss == ServiceLogLoss::default()
            && self.retry_at.is_none()
        {
            self.terminal = true;
        }
        ServiceLogWriterFlush {
            complete: self.terminal
                && self.terminal_error.is_none()
                && unflushed_chunks == 0
                && unflushed_bytes == 0
                && unflushed_loss == ServiceLogLoss::default(),
            unflushed_chunks,
            unflushed_bytes,
            unflushed_loss,
            terminal_error: self.terminal_error,
        }
    }

    /// Requests shutdown after the parent has quiesced event production.
    ///
    /// [`Self::final_flush`] still drains records already present in the
    /// bounded queue; the parent owns the ordering that prevents new records
    /// from arriving while that flush runs.
    pub const fn shutdown(&mut self) {
        self.shutdown_requested = true;
    }

    fn prepare_batch(&mut self) {
        let snapshot = self.buffer.snapshot();
        let has_loss = snapshot.loss != self.flushed_loss;
        if snapshot.queued_chunks == 0 && !has_loss {
            return;
        }
        let records = self
            .buffer
            .drain(MAX_SERVICE_LOG_QUEUE_CHUNKS, MAX_SERVICE_LOG_QUEUE_BYTES);
        if records.is_empty() && !has_loss {
            return;
        }
        self.pending = Some(GatewayServiceLogAppendBatch {
            records,
            loss: snapshot.loss,
        });
    }

    fn unflushed_counts(&self) -> (usize, usize) {
        let pending = self.pending.as_ref().map_or((0, 0), |batch| {
            (
                batch.records.len(),
                batch.records.iter().map(|record| record.bytes.len()).sum(),
            )
        });
        let queued = self.buffer.snapshot();
        (
            pending
                .0
                .saturating_add(queued.queued_chunks)
                .saturating_add(self.terminal_discarded.0),
            pending
                .1
                .saturating_add(queued.queued_bytes)
                .saturating_add(self.terminal_discarded.1),
        )
    }
}

const fn loss_difference(current: ServiceLogLoss, acknowledged: ServiceLogLoss) -> ServiceLogLoss {
    ServiceLogLoss {
        oversized_chunks: current
            .oversized_chunks
            .saturating_sub(acknowledged.oversized_chunks),
        oversized_bytes: current
            .oversized_bytes
            .saturating_sub(acknowledged.oversized_bytes),
        queue_full_chunks: current
            .queue_full_chunks
            .saturating_sub(acknowledged.queue_full_chunks),
        queue_full_bytes: current
            .queue_full_bytes
            .saturating_sub(acknowledged.queue_full_bytes),
        busy_chunks: current.busy_chunks.saturating_sub(acknowledged.busy_chunks),
        busy_bytes: current.busy_bytes.saturating_sub(acknowledged.busy_bytes),
        provider_lagged_events: current
            .provider_lagged_events
            .saturating_sub(acknowledged.provider_lagged_events),
        sequence_exhausted_chunks: current
            .sequence_exhausted_chunks
            .saturating_sub(acknowledged.sequence_exhausted_chunks),
        sequence_exhausted_bytes: current
            .sequence_exhausted_bytes
            .saturating_sub(acknowledged.sequence_exhausted_bytes),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    use ::time::OffsetDateTime;
    use async_trait::async_trait;
    use tokio::time::{self, Duration};
    use uuid::Uuid;
    use vm_trait::LogStream;

    use super::*;
    use crate::GatewayServiceLogAppendOutcome;

    struct FakeStore {
        results:
            Mutex<VecDeque<Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>>>,
        batches: Mutex<Vec<GatewayServiceLogAppendBatch>>,
        delay_millis: Arc<AtomicU64>,
    }

    #[async_trait]
    impl GatewayServiceLogStore for FakeStore {
        async fn append_batch(
            &self,
            _lease: &GatewayServiceInstanceLease,
            _owner: &GatewayServiceOwner,
            batch: GatewayServiceLogAppendBatch,
        ) -> Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError> {
            self.batches.lock().expect("batches lock").push(batch);
            let delay_millis = self.delay_millis.load(Ordering::Relaxed);
            if delay_millis != 0 {
                time::sleep(Duration::from_millis(delay_millis)).await;
            }
            self.results
                .lock()
                .expect("results lock")
                .pop_front()
                .unwrap_or_else(|| Ok(GatewayServiceLogAppendOutcome::default()))
        }
    }

    fn identity() -> GatewayServiceInstanceLease {
        GatewayServiceInstanceLease {
            identity: crate::GatewayServiceIdentity {
                instance_id: Uuid::new_v4(),
                gateway_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            },
            owner_host_id: "test-host".to_owned(),
            owner_uuid: Uuid::new_v4(),
            fencing_token: 1,
            state: crate::GatewayServiceInstanceState::Ready,
            vm_id: "test-vm".to_owned(),
            lease_expires_at: OffsetDateTime::now_utc() + ::time::Duration::hours(1),
            heartbeat_at: OffsetDateTime::now_utc(),
        }
    }

    fn writer(
        results: Vec<Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>>,
    ) -> (
        ServiceLogWriter<FakeStore>,
        Arc<FakeStore>,
        ServiceLogBufferHandle,
    ) {
        writer_with_delay(results, Duration::ZERO)
    }

    fn writer_with_delay(
        results: Vec<Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError>>,
        delay: Duration,
    ) -> (
        ServiceLogWriter<FakeStore>,
        Arc<FakeStore>,
        ServiceLogBufferHandle,
    ) {
        let lease = identity();
        let owner =
            GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid).expect("owner");
        let store = Arc::new(FakeStore {
            results: Mutex::new(results.into_iter().collect()),
            batches: Mutex::new(Vec::new()),
            delay_millis: Arc::new(AtomicU64::new(
                u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            )),
        });
        let buffer = ServiceLogBufferHandle::new();
        let writer = ServiceLogWriter::new(
            buffer.clone(),
            Arc::clone(&store),
            lease,
            owner,
            ServiceLogWriterPolicy::new(
                Duration::from_millis(20),
                Duration::from_millis(5),
                Duration::from_millis(20),
            ),
        )
        .expect("writer");
        (writer, store, buffer)
    }

    #[tokio::test]
    async fn appends_one_bounded_batch_and_loss_only_updates_are_not_duplicated() {
        let (mut writer, store, buffer) = writer(vec![Ok(GatewayServiceLogAppendOutcome {
            accepted_chunks: 1,
            ..GatewayServiceLogAppendOutcome::default()
        })]);
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"one");
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::Appended { .. }
        ));
        buffer.record_provider_lag(3);
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::Appended { .. }
        ));
        assert!(matches!(writer.poll().await, ServiceLogWriterPoll::Idle));
        let batches = store.batches.lock().expect("batches lock");
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].records.len(), 1);
        assert_eq!(batches[1].records.len(), 0);
        assert_eq!(batches[1].loss.provider_lagged_events, 3);
        drop(batches);
    }

    #[tokio::test]
    async fn unavailable_keeps_same_batch_and_backoff_is_bounded() {
        let (mut writer, store, buffer) = writer(vec![
            Err(GatewayServiceLogStoreError::Unavailable),
            Ok(GatewayServiceLogAppendOutcome::default()),
        ]);
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"same");
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::RetryScheduled { .. }
        ));
        assert_eq!(writer.snapshot().queued_chunks, 0);
        time::sleep(Duration::from_millis(6)).await;
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::Appended { .. }
        ));
        let batches = store.batches.lock().expect("batches lock");
        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches[0].records[0].sequence,
            batches[1].records[0].sequence
        );
        assert_eq!(batches[0].records[0].bytes, batches[1].records[0].bytes);
        drop(batches);
    }

    #[tokio::test]
    async fn dropped_poll_retains_batch_and_final_flush_reports_deadline() {
        let (mut first_writer, _store, buffer) =
            writer(vec![Err(GatewayServiceLogStoreError::Unavailable)]);
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"pending");
        let poll = tokio::spawn(async move { first_writer.poll().await });
        let _ = poll.await.expect("poll");
        let (mut writer_instance, _store, buffer) =
            writer(vec![Err(GatewayServiceLogStoreError::Unavailable)]);
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"pending");
        let _ = writer_instance.poll().await;
        let flush = writer_instance
            .final_flush(Instant::now() + Duration::from_millis(2))
            .await;
        assert!(!flush.complete);
        assert_eq!((flush.unflushed_chunks, flush.unflushed_bytes), (1, 7));
    }

    #[tokio::test]
    async fn cancelling_in_flight_append_retains_the_same_batch_for_retry() {
        let (mut writer, store, buffer) = writer_with_delay(
            vec![Ok(GatewayServiceLogAppendOutcome::default())],
            Duration::from_millis(50),
        );
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"retry");
        assert!(
            time::timeout(Duration::from_millis(2), writer.poll())
                .await
                .is_err()
        );
        store.delay_millis.store(0, Ordering::Relaxed);
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::Appended { .. }
        ));
        let batches = store.batches.lock().expect("batches lock");
        assert_eq!(batches.len(), 2);
        assert_eq!(
            batches[0].records[0].sequence,
            batches[1].records[0].sequence
        );
        assert_eq!(batches[0].records[0].bytes, batches[1].records[0].bytes);
        drop(batches);
    }

    #[tokio::test]
    async fn capacity_discards_only_one_batch_and_allows_later_records() {
        let (mut writer, _store, buffer) = writer(vec![
            Err(GatewayServiceLogStoreError::Capacity),
            Ok(GatewayServiceLogAppendOutcome::default()),
        ]);
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"drop");
        assert_eq!(
            writer.poll().await,
            ServiceLogWriterPoll::Capacity {
                discarded_chunks: 1,
                discarded_bytes: 4
            }
        );
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"keep");
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::Appended { .. }
        ));
    }

    #[tokio::test]
    async fn deadline_keeps_empty_loss_batch_for_later_retry() {
        let (mut writer, store, buffer) = writer_with_delay(
            vec![Ok(GatewayServiceLogAppendOutcome::default())],
            Duration::from_millis(50),
        );
        buffer.record_provider_lag(1);
        let flush = writer
            .final_flush(Instant::now() + Duration::from_millis(2))
            .await;
        assert!(!flush.complete);
        assert_eq!(flush.unflushed_chunks, 0);
        assert_eq!(flush.unflushed_bytes, 0);
        assert_eq!(flush.unflushed_loss.provider_lagged_events, 1);
        assert!(flush.terminal_error.is_none());

        store.delay_millis.store(0, Ordering::Relaxed);
        assert!(matches!(
            writer.poll().await,
            ServiceLogWriterPoll::Appended { .. }
        ));
        let completed = writer
            .final_flush(Instant::now() + Duration::from_millis(10))
            .await;
        assert!(completed.complete);
    }

    #[tokio::test]
    async fn terminal_results_stop_without_retrying() {
        let (mut writer, store, buffer) =
            writer(vec![Err(GatewayServiceLogStoreError::StaleLease)]);
        buffer.try_record(LogStream::Stdout, OffsetDateTime::UNIX_EPOCH, b"gone");
        assert_eq!(
            writer.poll().await,
            ServiceLogWriterPoll::Terminated {
                error: Some(GatewayServiceLogStoreError::StaleLease)
            }
        );
        assert_eq!(
            writer.poll().await,
            ServiceLogWriterPoll::Terminated {
                error: Some(GatewayServiceLogStoreError::StaleLease)
            }
        );
        assert_eq!(store.batches.lock().expect("batches lock").len(), 1);
    }
}
