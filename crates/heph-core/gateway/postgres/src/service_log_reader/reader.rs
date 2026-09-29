use super::{
    AuthenticatedIdentity, AuthorizationDecision, CandidateRow, GatewayServiceLogProjectMetadata,
    GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata, GatewayServiceLogReadPage,
    GatewayServiceLogReadRequest, GatewayServiceLogReadScope, GatewayServiceLogReaderError,
    ObjectRef, ObjectType, PayloadRow, Permission, PgPool, Postgres, PostgresMelangeAuthorizer,
    ProjectUsageRow, Subject, Transaction, audit_decision, begin_actor_transaction,
    begin_repeatable_read_actor_transaction, fetch_metadata, history_incomplete, payload_records,
    select_sequences,
};
use std::sync::Arc;

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

    /// Reads project-wide metadata-cap loss counters without exposing payloads.
    ///
    /// The project `CanRead` permission is the authorization boundary for this
    /// aggregate. Gateway-only authority is insufficient because the counters
    /// include losses from every gateway in the project.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, validation, or availability error without
    /// exposing SQL or payload details.
    pub async fn get_project_metadata(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: uuid::Uuid,
    ) -> Result<GatewayServiceLogProjectMetadata, GatewayServiceLogReaderError> {
        if project_id.is_nil() {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        if !self
            .authorize_object(
                &mut transaction,
                identity,
                ObjectRef::new(ObjectType::Project, project_id),
            )
            .await?
        {
            transaction
                .commit()
                .await
                .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
            return Err(GatewayServiceLogReaderError::Denied);
        }
        let usage = sqlx::query_as::<_, ProjectUsageRow>(
            "SELECT storage_dropped_chunks, storage_dropped_bytes
               FROM gateway_service_log_project_usage
              WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayServiceLogReaderError::Unavailable)?;
        usage.map_or_else(
            || Ok(GatewayServiceLogProjectMetadata::default()),
            ProjectUsageRow::into_metadata,
        )
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
            if !self.authorize_object(transaction, identity, object).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn authorize_object(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        object: ObjectRef,
    ) -> Result<bool, GatewayServiceLogReaderError> {
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
        Ok(decision == AuthorizationDecision::Allow)
    }
}

async fn commit_not_found(transaction: Transaction<'_, Postgres>) -> GatewayServiceLogReaderError {
    if transaction.commit().await.is_ok() {
        GatewayServiceLogReaderError::NotFound
    } else {
        GatewayServiceLogReaderError::Unavailable
    }
}
