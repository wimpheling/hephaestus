//! Authorized `PostgreSQL` reads for persistent service log epoch metadata.

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use gateway_edge::{GatewayServiceLogReadMetadata, GatewayServiceLogReadScope};
use identity_domain::AuthenticatedIdentity;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::sync::Arc;
use thiserror::Error;

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

        let row = sqlx::query_as::<_, LogMetadataRow>(
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
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;

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
