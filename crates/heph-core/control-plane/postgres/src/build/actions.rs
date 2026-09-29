use super::{
    BUILD_RETRY_REQUESTED_SUBJECT, BUILD_VERIFY_REQUESTED_SUBJECT, BuildActionError,
    BuildApplication, BuildError, BuildState, RequestedBuild,
};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

impl BuildApplication {
    /// Revalidates the build before reporting whether retry is possible.
    ///
    /// Queues a retry through the committed forge outbox. The trusted build
    /// worker owns execution reset and records the archived attempt.
    pub async fn retry_build(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<RequestedBuild, BuildActionError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        let row: Option<(Uuid, String, OffsetDateTime)> = sqlx::query_as(
            "SELECT id, state, created_at
               FROM build_requests
              WHERE id = $1
              FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        let Some((id, state, created_at)) = row else {
            return Err(BuildActionError::Application(BuildError::NotFound));
        };
        if !matches!(state.as_str(), "failed" | "cancelled") {
            return Err(BuildActionError::RetryNotAllowed);
        }
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "UPDATE build_requests
                SET state = 'queued', started_at = NULL, completed_at = NULL,
                    diagnostics = '[]'::jsonb
              WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        let event_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO outbox
                (id, aggregate_type, aggregate_id, subject, event_type, payload,
                 occurred_at)
             VALUES ($1, 'forge', $2, $3, 'build.retry_requested.v1', $4, $5)",
        )
        .bind(event_id)
        .bind(id)
        .bind(BUILD_RETRY_REQUESTED_SUBJECT)
        .bind(json!({
            "schema_version": 1,
            "message_id": event_id,
            "idempotency_key": event_id,
            "request_id": identity.request_id,
            "build_request_id": id,
        }))
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        transaction
            .commit()
            .await
            .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        Ok(RequestedBuild {
            id,
            state: BuildState::Queued,
            created_at,
            updated_at: now,
        })
    }

    /// Queues a verification rebuild over the same immutable build inputs.
    pub async fn rebuild_for_verification(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<RequestedBuild, BuildActionError> {
        let build = self
            .get_build(identity, id)
            .await
            .map_err(BuildActionError::Application)?;
        if build.state != BuildState::Succeeded {
            return Err(BuildActionError::VerificationNotAllowed);
        }
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        // The verification command leaves the immutable build state unchanged,
        // but clients still need a committed build event as the mutation
        // receipt and LiveView invalidation. Touching diagnostics deliberately
        // invokes the build-request application-event trigger without
        // fabricating a lifecycle transition.
        sqlx::query(
            "UPDATE build_requests
                SET diagnostics = diagnostics
              WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        let event_id = Uuid::new_v4();
        let now = OffsetDateTime::now_utc();
        sqlx::query(
            "INSERT INTO outbox
                (id, aggregate_type, aggregate_id, subject, event_type, payload,
                 occurred_at)
             VALUES ($1, 'forge', $2, $3, 'build.verify_requested.v1', $4, $5)",
        )
        .bind(event_id)
        .bind(id)
        .bind(BUILD_VERIFY_REQUESTED_SUBJECT)
        .bind(json!({
            "schema_version": 1,
            "message_id": event_id,
            "idempotency_key": event_id,
            "request_id": identity.request_id,
            "build_request_id": id,
        }))
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        transaction
            .commit()
            .await
            .map_err(|error| BuildActionError::Application(BuildError::Persistence(error)))?;
        Ok(RequestedBuild {
            id: build.id,
            state: build.state,
            created_at: build.created_at,
            updated_at: now,
        })
    }
}
