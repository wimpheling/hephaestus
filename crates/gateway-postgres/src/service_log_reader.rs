//! Authorized `PostgreSQL` reads for persistent service log epoch metadata.

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{
    PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction,
    begin_repeatable_read_actor_transaction,
};
use gateway_edge::{
    GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata, GatewayServiceLogReadPage,
    GatewayServiceLogReadRecord, GatewayServiceLogReadRequest, GatewayServiceLogReadScope,
    MAX_SERVICE_LOG_CHUNK_BYTES, MAX_SERVICE_LOG_READ_PAGE_BYTES,
};
use identity_domain::AuthenticatedIdentity;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::sync::Arc;
use thiserror::Error;
use time::OffsetDateTime;
use vm_trait::LogStream;

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

async fn fetch_metadata(
    transaction: &mut Transaction<'_, Postgres>,
    scope: GatewayServiceLogReadScope,
) -> Result<Option<LogMetadataRow>, GatewayServiceLogReaderError> {
    sqlx::query_as::<_, LogMetadataRow>(
        "SELECT instance.fencing_token AS current_fencing_token,
                epoch.fencing_token AS epoch_fencing_token,
                epoch.acknowledged_through,
                epoch.retained_bytes,
                epoch.retained_chunks,
                epoch.producer_dropped_chunks,
                epoch.producer_dropped_bytes,
                epoch.provider_lagged_events,
                epoch.storage_dropped_chunks,
                epoch.storage_dropped_bytes,
                epoch.evicted_chunks,
                epoch.evicted_bytes,
                min(chunk.sequence) AS earliest_retained_sequence
           FROM gateway_service_instances AS instance
           JOIN gateways AS gateway
             ON gateway.id = instance.gateway_id
           LEFT JOIN gateway_service_log_epochs AS epoch
             ON epoch.instance_id = instance.id
            AND epoch.gateway_id = instance.gateway_id
            AND epoch.revision_id = instance.revision_id
            AND epoch.project_id = gateway.project_id
            AND epoch.fencing_token = $5
           LEFT JOIN gateway_service_log_chunks AS chunk
             ON chunk.instance_id = epoch.instance_id
            AND chunk.gateway_id = epoch.gateway_id
            AND chunk.revision_id = epoch.revision_id
            AND chunk.project_id = epoch.project_id
            AND chunk.fencing_token = epoch.fencing_token
          WHERE instance.id = $1
            AND instance.gateway_id = $2
            AND instance.revision_id = $3
            AND gateway.project_id = $4
          GROUP BY instance.fencing_token,
                   epoch.fencing_token,
                   epoch.acknowledged_through,
                   epoch.retained_bytes,
                   epoch.retained_chunks,
                   epoch.producer_dropped_chunks,
                   epoch.producer_dropped_bytes,
                   epoch.provider_lagged_events,
                   epoch.storage_dropped_chunks,
                   epoch.storage_dropped_bytes,
                   epoch.evicted_chunks,
                   epoch.evicted_bytes",
    )
    .bind(scope.instance_id)
    .bind(scope.gateway_id)
    .bind(scope.revision_id)
    .bind(scope.project_id)
    .bind(scope.fencing_token)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| GatewayServiceLogReaderError::Unavailable)
}

fn select_sequences(
    candidates: &[CandidateRow],
    limit: u16,
) -> Result<(Vec<i64>, bool), GatewayServiceLogReaderError> {
    let mut selected = Vec::with_capacity(usize::from(limit));
    let mut total_bytes = 0_usize;
    for candidate in candidates {
        let byte_len = usize::try_from(candidate.byte_len)
            .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?;
        if candidate.sequence < 0 || byte_len > MAX_SERVICE_LOG_CHUNK_BYTES {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        if selected.len() >= usize::from(limit)
            || total_bytes.saturating_add(byte_len) > MAX_SERVICE_LOG_READ_PAGE_BYTES
        {
            break;
        }
        total_bytes = total_bytes.saturating_add(byte_len);
        selected.push(candidate.sequence);
    }
    let has_more = selected.len() < candidates.len();
    Ok((selected, has_more))
}

fn payload_records(
    rows: Vec<PayloadRow>,
    selected_sequences: &[i64],
) -> Result<Vec<GatewayServiceLogReadRecord>, GatewayServiceLogReaderError> {
    if rows.len() != selected_sequences.len() {
        return Err(GatewayServiceLogReaderError::Unavailable);
    }
    let mut total_bytes = 0_usize;
    let mut records = Vec::with_capacity(rows.len());
    for (row, expected_sequence) in rows.into_iter().zip(selected_sequences) {
        if row.sequence != *expected_sequence || row.bytes.len() > MAX_SERVICE_LOG_CHUNK_BYTES {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        total_bytes = total_bytes.saturating_add(row.bytes.len());
        if total_bytes > MAX_SERVICE_LOG_READ_PAGE_BYTES {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        records.push(GatewayServiceLogReadRecord {
            sequence: u64::try_from(row.sequence)
                .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?,
            stream: parse_stream(&row.stream)?,
            observed_at: row.observed_at,
            stored_at: row.stored_at,
            bytes: row.bytes,
        });
    }
    Ok(records)
}

fn parse_stream(stream: &str) -> Result<LogStream, GatewayServiceLogReaderError> {
    match stream {
        "stdout" => Ok(LogStream::Stdout),
        "stderr" => Ok(LogStream::Stderr),
        _ => Err(GatewayServiceLogReaderError::InvalidArgument),
    }
}

fn history_incomplete(
    metadata: &GatewayServiceLogReadMetadata,
    after: Option<GatewayServiceLogReadCursor>,
) -> bool {
    let expected = after.map_or(0, |cursor| cursor.sequence().saturating_add(1));
    metadata.earliest_retained_sequence.map_or_else(
        || {
            metadata
                .acknowledged_through
                .is_some_and(|acknowledged| expected <= acknowledged)
        },
        |earliest| expected < earliest,
    )
}

/// Application-role `PostgreSQL` reader for one service log epoch's metadata.
#[derive(Clone)]
pub struct PostgresGatewayServiceLogReader {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

impl PostgresGatewayServiceLogReader {
    /// Creates a reader over the application pool. The pool must select
    /// `hephaestus_app`; worker-owned append and maintenance pools are not
    /// valid for this adapter.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self { pool, authorizer }
    }

    /// Reads metadata for one exact instance and fencing epoch.
    ///
    /// Historical epochs at or below the current instance fence remain
    /// readable to currently authorized project members. An exact instance
    /// without an epoch returns an empty metadata value; an absent, mismatched,
    /// or future-fenced instance returns `NotFound`.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, absence, validation, or availability
    /// error without exposing SQL or payload details.
    pub async fn get_epoch_metadata(
        &self,
        identity: &AuthenticatedIdentity,
        scope: GatewayServiceLogReadScope,
    ) -> Result<GatewayServiceLogReadMetadata, GatewayServiceLogReaderError> {
        scope
            .validate()
            .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?;
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;

        if !self.authorize(&mut transaction, identity, scope).await? {
            transaction
                .commit()
                .await
                .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
            return Err(GatewayServiceLogReaderError::Denied);
        }

        let row = fetch_metadata(&mut transaction, scope).await?;

        let Some(row) = row else {
            if transaction.commit().await.is_err() {
                return Err(GatewayServiceLogReaderError::Unavailable);
            }
            return Err(GatewayServiceLogReaderError::NotFound);
        };

        if scope.fencing_token > row.current_fencing_token {
            return Err(commit_not_found(transaction).await);
        }
        let metadata = row.into_metadata()?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        Ok(metadata)
    }

    /// Reads one bounded ordered payload page and its durable metadata.
    ///
    /// The metadata, candidate lengths, and selected payload rows share one
    /// repeatable-read snapshot. Candidate SQL transfers only sequence and
    /// byte-length metadata; raw bytes are selected by exact sequence IDs only
    /// after the record and 512 KiB budgets have been enforced.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, absence, validation, or availability
    /// error without exposing SQL or payload details.
    pub async fn get_page(
        &self,
        identity: &AuthenticatedIdentity,
        request: GatewayServiceLogReadRequest,
    ) -> Result<GatewayServiceLogReadPage, GatewayServiceLogReaderError> {
        request
            .validate()
            .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?;
        let scope = request.scope;
        let mut transaction = begin_repeatable_read_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        if !self.authorize(&mut transaction, identity, scope).await? {
            transaction
                .commit()
                .await
                .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
            return Err(GatewayServiceLogReaderError::Denied);
        }
        let Some(row) = fetch_metadata(&mut transaction, scope).await? else {
            return Err(commit_not_found(transaction).await);
        };
        if scope.fencing_token > row.current_fencing_token {
            return Err(commit_not_found(transaction).await);
        }
        let metadata = row.into_metadata()?;
        let after_sequence = request.after.map_or(-1, |cursor| {
            i64::try_from(cursor.sequence()).unwrap_or(i64::MAX)
        });
        let candidates = sqlx::query_as::<_, CandidateRow>(
            "SELECT sequence, octet_length(bytes)::bigint AS byte_len
               FROM gateway_service_log_chunks
              WHERE instance_id = $1
                AND gateway_id = $2
                AND revision_id = $3
                AND project_id = $4
                AND fencing_token = $5
                AND sequence > $6
              ORDER BY sequence
              LIMIT $7",
        )
        .bind(scope.instance_id)
        .bind(scope.gateway_id)
        .bind(scope.revision_id)
        .bind(scope.project_id)
        .bind(scope.fencing_token)
        .bind(after_sequence)
        .bind(i64::from(request.limit) + 1)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        let (selected_sequences, has_more) = select_sequences(&candidates, request.limit)?;
        let records = if selected_sequences.is_empty() {
            Vec::new()
        } else {
            let rows = sqlx::query_as::<_, PayloadRow>(
                "SELECT sequence, stream, observed_at, stored_at, bytes
                   FROM gateway_service_log_chunks
                  WHERE instance_id = $1
                    AND gateway_id = $2
                    AND revision_id = $3
                    AND project_id = $4
                    AND fencing_token = $5
                    AND sequence = ANY($6::bigint[])
                  ORDER BY sequence",
            )
            .bind(scope.instance_id)
            .bind(scope.gateway_id)
            .bind(scope.revision_id)
            .bind(scope.project_id)
            .bind(scope.fencing_token)
            .bind(&selected_sequences)
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
            payload_records(rows, &selected_sequences)?
        };
        let history_incomplete = history_incomplete(&metadata, request.after);
        let next_after = if has_more {
            let sequence = selected_sequences
                .last()
                .copied()
                .ok_or(GatewayServiceLogReaderError::InvalidArgument)?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?;
            Some(
                GatewayServiceLogReadCursor::new(scope, sequence)
                    .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?,
            )
        } else {
            None
        };
        transaction
            .commit()
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        Ok(GatewayServiceLogReadPage {
            records,
            metadata,
            history_incomplete,
            next_after,
        })
    }

    async fn authorize(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        scope: GatewayServiceLogReadScope,
    ) -> Result<bool, GatewayServiceLogReaderError> {
        for object in [
            ObjectRef::new(ObjectType::Project, scope.project_id),
            ObjectRef::new(ObjectType::Gateway, scope.gateway_id),
        ] {
            let decision = self
                .authorizer
                .check(
                    transaction,
                    Subject::User(identity.user_id),
                    Permission::CanRead,
                    object,
                )
                .await
                .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
            audit_decision(
                transaction,
                identity.user_id,
                Permission::CanRead,
                object,
                decision,
                identity.request_id,
            )
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
            if decision != AuthorizationDecision::Allow {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

async fn commit_not_found(transaction: Transaction<'_, Postgres>) -> GatewayServiceLogReaderError {
    if transaction.commit().await.is_ok() {
        GatewayServiceLogReaderError::NotFound
    } else {
        GatewayServiceLogReaderError::Unavailable
    }
}

#[derive(Debug, FromRow)]
struct LogMetadataRow {
    current_fencing_token: i64,
    epoch_fencing_token: Option<i64>,
    acknowledged_through: Option<i64>,
    retained_bytes: Option<i64>,
    retained_chunks: Option<i64>,
    producer_dropped_chunks: Option<i64>,
    producer_dropped_bytes: Option<i64>,
    provider_lagged_events: Option<i64>,
    storage_dropped_chunks: Option<i64>,
    storage_dropped_bytes: Option<i64>,
    evicted_chunks: Option<i64>,
    evicted_bytes: Option<i64>,
    earliest_retained_sequence: Option<i64>,
}

#[derive(Debug, FromRow)]
struct CandidateRow {
    sequence: i64,
    byte_len: i64,
}

#[derive(FromRow)]
struct PayloadRow {
    sequence: i64,
    stream: String,
    observed_at: OffsetDateTime,
    stored_at: OffsetDateTime,
    bytes: Vec<u8>,
}

impl LogMetadataRow {
    fn into_metadata(self) -> Result<GatewayServiceLogReadMetadata, GatewayServiceLogReaderError> {
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

fn required(value: Option<i64>) -> Result<i64, GatewayServiceLogReaderError> {
    value.ok_or(GatewayServiceLogReaderError::InvalidArgument)
}

const fn nonnegative_or_none(
    value: Option<i64>,
) -> Result<Option<i64>, GatewayServiceLogReaderError> {
    match value {
        Some(value) if value < -1 => Err(GatewayServiceLogReaderError::InvalidArgument),
        other => Ok(other),
    }
}

fn to_u64(value: i64) -> Result<u64, GatewayServiceLogReaderError> {
    u64::try_from(value).map_err(|_| GatewayServiceLogReaderError::InvalidArgument)
}
