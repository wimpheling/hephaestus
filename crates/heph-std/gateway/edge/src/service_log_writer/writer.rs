use std::{fmt, sync::Arc, time::Duration};

use tokio::time::{self, Instant};

use super::{
    ServiceLogWriterError, ServiceLogWriterFlush, ServiceLogWriterPolicy, ServiceLogWriterPoll,
};
use crate::{
    GatewayServiceInstanceLease, GatewayServiceLogAppendBatch, GatewayServiceLogStore,
    GatewayServiceLogStoreError, GatewayServiceOwner, MAX_SERVICE_LOG_QUEUE_BYTES,
    MAX_SERVICE_LOG_QUEUE_CHUNKS, ServiceLogBufferHandle, ServiceLogBufferSnapshot, ServiceLogLoss,
};

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
