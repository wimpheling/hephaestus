//! Bounded authorized service-log read contracts.

use std::fmt;

use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

use crate::GatewayEdgeError;

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
