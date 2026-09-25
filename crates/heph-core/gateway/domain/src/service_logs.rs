//! Bounded, opt-in application log capture for one service instance.

use std::fmt;

use async_trait::async_trait;
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

use crate::{GatewayEdgeError, GatewayServiceInstanceLease, GatewayServiceOwner};

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
/// Maximum number of records returned by one authorized log page.
pub const MAX_SERVICE_LOG_READ_PAGE_RECORDS: u16 = 100;
/// Maximum payload bytes returned by one authorized log page.
pub const MAX_SERVICE_LOG_READ_PAGE_BYTES: usize = 512 * 1024;

/// Exact durable scope of one service log fencing epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GatewayServiceLogReadScope {
    /// Owning project identity.
    pub project_id: Uuid,
    /// Gateway identity.
    pub gateway_id: Uuid,
    /// Immutable gateway revision identity.
    pub revision_id: Uuid,
    /// Durable service instance identity.
    pub instance_id: Uuid,
    /// Positive fencing epoch of the instance.
    pub fencing_token: i64,
}

impl GatewayServiceLogReadScope {
    /// Creates an exact, non-nil service log scope.
    ///
    /// # Errors
    ///
    /// Returns a contract error when an identity is nil or the fencing token
    /// is not positive.
    pub fn new(
        project_id: Uuid,
        gateway_id: Uuid,
        revision_id: Uuid,
        instance_id: Uuid,
        fencing_token: i64,
    ) -> Result<Self, GatewayEdgeError> {
        let scope = Self {
            project_id,
            gateway_id,
            revision_id,
            instance_id,
            fencing_token,
        };
        scope.validate()?;
        Ok(scope)
    }

    /// Validates a scope assembled by an internal caller.
    ///
    /// # Errors
    ///
    /// Returns a contract error when an identity is nil or the fencing token
    /// is not positive.
    pub const fn validate(self) -> Result<(), GatewayEdgeError> {
        if self.project_id.is_nil()
            || self.gateway_id.is_nil()
            || self.revision_id.is_nil()
            || self.instance_id.is_nil()
            || self.fencing_token <= 0
        {
            return Err(GatewayEdgeError::Contract(
                "invalid gateway service log read scope",
            ));
        }
        Ok(())
    }
}

/// Scope-bound sequence cursor for an exact service log fencing epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GatewayServiceLogReadCursor {
    scope: GatewayServiceLogReadScope,
    sequence: u64,
}

impl GatewayServiceLogReadCursor {
    /// Creates a cursor for one exact scope and sequence.
    ///
    /// # Errors
    ///
    /// Returns a contract error when the scope is invalid or the sequence
    /// cannot be represented by the `PostgreSQL` `bigint` sequence column.
    pub fn new(scope: GatewayServiceLogReadScope, sequence: u64) -> Result<Self, GatewayEdgeError> {
        scope.validate()?;
        if sequence > i64::MAX as u64 {
            return Err(GatewayEdgeError::Contract(
                "gateway service log cursor sequence is too large",
            ));
        }
        Ok(Self { scope, sequence })
    }

    /// Returns the exact scope bound into this cursor.
    #[must_use]
    pub const fn scope(self) -> GatewayServiceLogReadScope {
        self.scope
    }

    /// Returns the exclusive sequence position.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

/// Bounded authorized read request for one exact service log epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLogReadRequest {
    /// Exact project/gateway/revision/instance/fence scope.
    pub scope: GatewayServiceLogReadScope,
    /// Maximum number of records requested.
    pub limit: u16,
    /// Exclusive sequence cursor, when resuming a page.
    pub after: Option<GatewayServiceLogReadCursor>,
}

impl GatewayServiceLogReadRequest {
    /// Creates a bounded scope-bound read request.
    ///
    /// # Errors
    ///
    /// Returns a contract error for malformed scope, page size, or a cursor
    /// bound to another scope.
    pub fn new(
        scope: GatewayServiceLogReadScope,
        limit: u16,
        after: Option<GatewayServiceLogReadCursor>,
    ) -> Result<Self, GatewayEdgeError> {
        let request = Self {
            scope,
            limit,
            after,
        };
        request.validate()?;
        Ok(request)
    }

    /// Validates a request assembled by an internal caller.
    ///
    /// # Errors
    ///
    /// Returns a contract error for malformed scope, page size, or a cursor
    /// bound to another scope.
    pub fn validate(self) -> Result<(), GatewayEdgeError> {
        self.scope.validate()?;
        if self.limit == 0 || self.limit > MAX_SERVICE_LOG_READ_PAGE_RECORDS {
            return Err(GatewayEdgeError::Contract(
                "invalid gateway service log read page",
            ));
        }
        if self
            .after
            .is_some_and(|cursor| cursor.scope() != self.scope)
        {
            return Err(GatewayEdgeError::Contract(
                "gateway service log cursor scope mismatch",
            ));
        }
        Ok(())
    }
}

/// One authorized service log payload returned by a reader.
#[derive(Clone)]
pub struct GatewayServiceLogReadRecord {
    /// Monotonic worker sequence within the exact fencing epoch.
    pub sequence: u64,
    /// Guest output stream.
    pub stream: LogStream,
    /// Host time at which the provider observed the chunk.
    pub observed_at: OffsetDateTime,
    /// Time at which the platform stored the chunk.
    pub stored_at: OffsetDateTime,
    /// Application-owned bytes. These are intentionally excluded from debug.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for GatewayServiceLogReadRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GatewayServiceLogReadRecord")
            .field("sequence", &self.sequence)
            .field("stream", &self.stream)
            .field("observed_at", &self.observed_at)
            .field("stored_at", &self.stored_at)
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

/// Durable loss and retention metadata for one exact log epoch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatewayServiceLogReadMetadata {
    /// Whether the exact epoch exists in durable metadata.
    pub epoch_present: bool,
    /// Highest sequence acknowledged, including evicted or dropped records.
    pub acknowledged_through: Option<u64>,
    /// Current retained payload bytes and rows.
    pub retained_bytes: u64,
    /// Current retained payload row count.
    pub retained_chunks: u64,
    /// Producer, provider, storage, and retention loss counters.
    pub producer_dropped_chunks: u64,
    /// Bytes reported lost by the producer.
    pub producer_dropped_bytes: u64,
    /// Provider events skipped before the collector received them.
    pub provider_lagged_events: u64,
    /// Chunks rejected by durable storage capacity.
    pub storage_dropped_chunks: u64,
    /// Bytes rejected by durable storage capacity.
    pub storage_dropped_bytes: u64,
    /// Payload rows removed by retention maintenance.
    pub evicted_chunks: u64,
    /// Bytes removed by retention maintenance.
    pub evicted_bytes: u64,
    /// Lowest retained sequence, when at least one payload remains.
    pub earliest_retained_sequence: Option<u64>,
}

/// Project-wide metadata-cap loss counters for authorized diagnostics.
///
/// These counters aggregate rejected submissions across all service log
/// epochs in the project. An ambiguous commit followed by a retry can count
/// the same submission more than once; they are not an exact event-loss
/// total. They remain after epoch payload and metadata retention cleanup.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GatewayServiceLogProjectMetadata {
    /// Whether a durable project usage row exists.
    pub usage_present: bool,
    /// Project-wide rejected submission chunk count.
    pub storage_dropped_chunks: u64,
    /// Project-wide rejected submission byte count.
    pub storage_dropped_bytes: u64,
}

/// Bounded page returned by an authorized service log reader.
#[derive(Debug, Clone)]
pub struct GatewayServiceLogReadPage {
    /// Payload rows in increasing sequence order.
    pub records: Vec<GatewayServiceLogReadRecord>,
    /// Durable loss and retention metadata for the requested epoch.
    pub metadata: GatewayServiceLogReadMetadata,
    /// Whether the requested cursor precedes retained payload history.
    /// This does not identify the exact cause or byte count of the gap.
    pub history_incomplete: bool,
    /// Scope-bound cursor for the next page, when more rows exist.
    pub next_after: Option<GatewayServiceLogReadCursor>,
}

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

/// Maximum number of projects returned by one maintenance enumeration page.
pub const MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE: u16 = 128;

/// Bounded keyset page for worker-owned service-log maintenance projects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenanceProjectPage {
    /// Return projects strictly after this stable project identity.
    pub after: Option<Uuid>,
    /// Maximum number of project identities to return.
    pub limit: u16,
}

impl GatewayServiceLogMaintenanceProjectPage {
    /// Creates a validated maintenance-project page.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLogMaintenanceError::InvalidArgument`] for a
    /// zero or oversized page, or a nil cursor.
    pub fn new(after: Option<Uuid>, limit: u16) -> Result<Self, GatewayServiceLogMaintenanceError> {
        let page = Self { after, limit };
        page.validate()?;
        Ok(page)
    }

    /// Validates a page assembled by an internal scheduler.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceLogMaintenanceError::InvalidArgument`] when
    /// the page limit or cursor is outside the bounded contract.
    pub fn validate(&self) -> Result<(), GatewayServiceLogMaintenanceError> {
        if self.limit == 0
            || self.limit > MAX_SERVICE_LOG_MAINTENANCE_PROJECT_PAGE_SIZE
            || self.after.is_some_and(|value| value.is_nil())
        {
            return Err(GatewayServiceLogMaintenanceError::InvalidArgument);
        }
        Ok(())
    }
}

/// Bounded UUID page returned by the worker-only maintenance index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceLogMaintenanceProjectPageResult {
    /// Project identities backed by durable log usage rows.
    pub projects: Vec<Uuid>,
    /// Cursor for the next keyset page, if one exists.
    pub next_after: Option<Uuid>,
}

/// Worker-only enumeration of projects that have durable service-log state.
#[async_trait]
pub trait GatewayServiceLogMaintenanceProjects: Send + Sync {
    /// Lists project usage rows in stable UUID order, including projects with
    /// no active service instance. A scheduler must keep failed projects
    /// eligible for a later bounded retry while advancing fairly, so one
    /// transient failure does not block the rest of a sweep.
    async fn list_projects(
        &self,
        page: GatewayServiceLogMaintenanceProjectPage,
    ) -> Result<GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceError>;
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
