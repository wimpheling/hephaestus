use super::{
    Error, FromRow, GatewayServiceLogProjectMetadata, GatewayServiceLogReadMetadata, OffsetDateTime,
};

/// Safe failures for authorized service-log metadata reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GatewayServiceLogReaderError {
    /// The actor cannot read the project or gateway.
    #[error("service log metadata is not authorized")]
    Denied,
    /// The exact instance or scope does not exist for this actor.
    #[error("service log metadata is not found")]
    NotFound,
    /// The supplied durable scope or stored metadata is invalid.
    #[error("service log metadata request is invalid")]
    InvalidArgument,
    /// The bounded authorization or metadata query could not complete.
    #[error("service log metadata is unavailable")]
    Unavailable,
}

#[derive(Debug, FromRow)]
pub(super) struct LogMetadataRow {
    pub(super) current_fencing_token: i64,
    pub(super) epoch_fencing_token: Option<i64>,
    pub(super) acknowledged_through: Option<i64>,
    pub(super) retained_bytes: Option<i64>,
    pub(super) retained_chunks: Option<i64>,
    pub(super) producer_dropped_chunks: Option<i64>,
    pub(super) producer_dropped_bytes: Option<i64>,
    pub(super) provider_lagged_events: Option<i64>,
    pub(super) storage_dropped_chunks: Option<i64>,
    pub(super) storage_dropped_bytes: Option<i64>,
    pub(super) evicted_chunks: Option<i64>,
    pub(super) evicted_bytes: Option<i64>,
    pub(super) earliest_retained_sequence: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(super) struct CandidateRow {
    pub(super) sequence: i64,
    pub(super) byte_len: i64,
}

#[derive(FromRow)]
pub(super) struct PayloadRow {
    pub(super) sequence: i64,
    pub(super) stream: String,
    pub(super) observed_at: OffsetDateTime,
    pub(super) stored_at: OffsetDateTime,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug, FromRow)]
pub(super) struct ProjectUsageRow {
    pub(super) storage_dropped_chunks: i64,
    pub(super) storage_dropped_bytes: i64,
}

impl ProjectUsageRow {
    pub(super) fn into_metadata(
        self,
    ) -> Result<GatewayServiceLogProjectMetadata, GatewayServiceLogReaderError> {
        Ok(GatewayServiceLogProjectMetadata {
            usage_present: true,
            storage_dropped_chunks: to_u64(self.storage_dropped_chunks)?,
            storage_dropped_bytes: to_u64(self.storage_dropped_bytes)?,
        })
    }
}

impl LogMetadataRow {
    pub(super) fn into_metadata(
        self,
    ) -> Result<GatewayServiceLogReadMetadata, GatewayServiceLogReaderError> {
        let Some(epoch_fencing_token) = self.epoch_fencing_token else {
            return Ok(GatewayServiceLogReadMetadata::default());
        };
        if epoch_fencing_token <= 0 {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        let acknowledged_through = nonnegative_or_none(self.acknowledged_through)?;
        Ok(GatewayServiceLogReadMetadata {
            epoch_present: true,
            acknowledged_through: match acknowledged_through {
                Some(-1) | None => None,
                Some(value) => Some(to_u64(value)?),
            },
            retained_bytes: to_u64(required(self.retained_bytes)?)?,
            retained_chunks: to_u64(required(self.retained_chunks)?)?,
            producer_dropped_chunks: to_u64(required(self.producer_dropped_chunks)?)?,
            producer_dropped_bytes: to_u64(required(self.producer_dropped_bytes)?)?,
            provider_lagged_events: to_u64(required(self.provider_lagged_events)?)?,
            storage_dropped_chunks: to_u64(required(self.storage_dropped_chunks)?)?,
            storage_dropped_bytes: to_u64(required(self.storage_dropped_bytes)?)?,
            evicted_chunks: to_u64(required(self.evicted_chunks)?)?,
            evicted_bytes: to_u64(required(self.evicted_bytes)?)?,
            earliest_retained_sequence: self.earliest_retained_sequence.map(to_u64).transpose()?,
        })
    }
}

pub(super) fn required(value: Option<i64>) -> Result<i64, GatewayServiceLogReaderError> {
    value.ok_or(GatewayServiceLogReaderError::InvalidArgument)
}

pub(super) const fn nonnegative_or_none(
    value: Option<i64>,
) -> Result<Option<i64>, GatewayServiceLogReaderError> {
    match value {
        Some(value) if value < -1 => Err(GatewayServiceLogReaderError::InvalidArgument),
        other => Ok(other),
    }
}

pub(super) fn to_u64(value: i64) -> Result<u64, GatewayServiceLogReaderError> {
    u64::try_from(value).map_err(|_| GatewayServiceLogReaderError::InvalidArgument)
}
