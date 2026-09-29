use super::mapping::insert_verification;
use super::notification::{
    ClaimedRegistryNotification, NewRegistryNotification, NotificationCompletion,
    RegistryNotificationReceipt, RegistryStoreError, Transition,
};
use super::parsing::{state_text, storage, validate_failure_code};
use super::prelude::*;
use super::rows::{NotificationRow, load_intent};
use super::store::PgRegistryStore;

// These public transition methods share the adapter's stable error contract;
// their individual SQL paths are documented by the surrounding lifecycle API.
#[allow(clippy::missing_errors_doc)]
impl PgRegistryStore {
    /// Claims a pending publication for a trusted publisher.
    pub async fn begin_publishing(
        &self,
        id: PublicationIntentId,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        self.transition(id, Transition::Begin).await
    }

    /// Returns an interrupted publishing attempt to the retryable pending state.
    pub async fn retry(
        &self,
        id: PublicationIntentId,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        self.transition(id, Transition::Retry).await
    }

    /// Records exact Zot verification evidence, idempotently.
    pub async fn record_verified(
        &self,
        id: PublicationIntentId,
        verification: VerifiedPublication,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let intent = load_intent(&mut transaction, id.as_uuid(), true).await?;
        let verified = intent
            .clone()
            .record_verified(verification.clone())
            .map_err(RegistryStoreError::Lifecycle)?;
        if matches!(
            intent.state(),
            PublicationState::Verified | PublicationState::Approved
        ) {
            transaction.commit().await.map_err(storage)?;
            return Ok(verified);
        }
        insert_verification(&mut transaction, id.as_uuid(), &verification).await?;
        sqlx::query(
            "UPDATE registry_publications
             SET state = 'verified', verified_at = now()
             WHERE id = $1 AND state IN ('pending', 'publishing')",
        )
        .bind(id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(verified)
    }

    /// Commits already verified evidence as an executable immutable approval.
    pub async fn approve(
        &self,
        id: PublicationIntentId,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        self.transition(id, Transition::Approve).await
    }

    /// Marks previously approved Zot content absent, causing consumers to fail closed.
    pub async fn mark_missing(
        &self,
        id: PublicationIntentId,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        self.transition(id, Transition::Missing).await
    }

    /// Restores a missing publication only after the exact immutable evidence is reverified.
    pub async fn restore_verified(
        &self,
        id: PublicationIntentId,
        verification: &VerifiedPublication,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let intent = load_intent(&mut transaction, id.as_uuid(), true).await?;
        let restored = intent
            .restore_verified(verification)
            .map_err(RegistryStoreError::Lifecycle)?;
        sqlx::query(
            "UPDATE registry_publications SET state = 'approved'
             WHERE id = $1 AND state = 'missing'",
        )
        .bind(id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(restored)
    }

    /// Retires an intent while retaining its immutable historical verification.
    pub async fn retire(
        &self,
        id: PublicationIntentId,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        self.transition(id, Transition::Retire).await
    }

    async fn transition(
        &self,
        id: PublicationIntentId,
        transition: Transition,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let intent = load_intent(&mut transaction, id.as_uuid(), true).await?;
        let next = transition.apply(intent.clone())?;
        if next.state() != intent.state() {
            let state = state_text(next.state());
            match transition {
                Transition::Approve => {
                    sqlx::query(
                        "UPDATE registry_publications
                         SET state = $2, approved_at = now() WHERE id = $1",
                    )
                    .bind(id.as_uuid())
                    .bind(state)
                    .execute(&mut *transaction)
                    .await
                    .map_err(storage)?;
                }
                Transition::Begin
                | Transition::Retry
                | Transition::Missing
                | Transition::Retire => {
                    sqlx::query("UPDATE registry_publications SET state = $2 WHERE id = $1")
                        .bind(id.as_uuid())
                        .bind(state)
                        .execute(&mut *transaction)
                        .await
                        .map_err(storage)?;
                }
            }
        }
        transaction.commit().await.map_err(storage)?;
        Ok(next)
    }

    /// Inserts a bounded notification observation, rejecting event-key reuse
    /// with a different body hash.
    pub async fn ingest_notification(
        &self,
        notification: NewRegistryNotification,
    ) -> Result<RegistryNotificationReceipt, RegistryStoreError> {
        notification.validate()?;
        let inserted = sqlx::query_as::<_, NotificationRow>(
            "INSERT INTO registry_notification_inbox (
                id, event_key, repository_path, action, target_digest,
                target_media_type, target_size, event_occurred_at, payload_sha256
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (event_key) DO NOTHING
             RETURNING id, event_key, repository_path, action, target_digest,
                 target_media_type, target_size, event_occurred_at, payload_sha256,
                 state, claim_token, lease_expires_at, failure_code, processed_at",
        )
        .bind(Uuid::new_v4())
        .bind(&notification.event_key)
        .bind(&notification.repository_path)
        .bind(notification.action.as_str())
        .bind(
            notification
                .target
                .as_ref()
                .map(|value| value.digest.as_str()),
        )
        .bind(
            notification
                .target
                .as_ref()
                .map(|value| value.media_type.as_str()),
        )
        .bind(Option::<i64>::None)
        .bind(notification.occurred_at)
        .bind(notification.payload_sha256.as_slice())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        let (row, duplicate) = match inserted {
            Some(row) => (row, false),
            None => (
                sqlx::query_as::<_, NotificationRow>(
                    "SELECT id, event_key, repository_path, action, target_digest,
                    target_media_type, target_size, event_occurred_at, payload_sha256,
                    state, claim_token, lease_expires_at, failure_code, processed_at
                 FROM registry_notification_inbox WHERE event_key = $1",
                )
                .bind(&notification.event_key)
                .fetch_one(&self.pool)
                .await
                .map_err(storage)?,
                true,
            ),
        };
        if row.payload_sha256 != notification.payload_sha256 {
            return Err(RegistryStoreError::Conflict);
        }
        let mut receipt = row.try_into_receipt()?;
        receipt.duplicate = duplicate;
        Ok(receipt)
    }

    /// Claims one notification for a bounded lease using `SKIP LOCKED`.
    pub async fn claim_notification(
        &self,
        lease: Duration,
    ) -> Result<Option<ClaimedRegistryNotification>, RegistryStoreError> {
        let lease_seconds = i64::try_from(lease.as_secs())
            .ok()
            .filter(|seconds| *seconds > 0 && *seconds <= 3_600)
            .ok_or(RegistryStoreError::Conflict)?;
        let claim_token = Uuid::new_v4();
        let row = sqlx::query_as::<_, NotificationRow>(
            "WITH candidate AS (
                SELECT id FROM registry_notification_inbox
                 WHERE state = 'pending' OR (state = 'claimed' AND lease_expires_at <= now())
                 ORDER BY received_at, id FOR UPDATE SKIP LOCKED LIMIT 1
             ) UPDATE registry_notification_inbox inbox
                  SET state = 'claimed', claim_token = $1,
                      lease_expires_at = now() + make_interval(secs => $2)
                 FROM candidate WHERE inbox.id = candidate.id
             RETURNING inbox.id, inbox.event_key, inbox.repository_path, inbox.action,
                 inbox.target_digest, inbox.target_media_type, inbox.target_size,
                 inbox.event_occurred_at, inbox.payload_sha256, inbox.state,
                 inbox.claim_token, inbox.lease_expires_at, inbox.failure_code,
                 inbox.processed_at",
        )
        .bind(claim_token)
        .bind(lease_seconds)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(NotificationRow::try_into_claimed).transpose()
    }

    /// Completes a claimed notification without changing any publication state.
    pub async fn complete_notification(
        &self,
        id: Uuid,
        claim_token: Uuid,
        outcome: NotificationCompletion,
    ) -> Result<(), RegistryStoreError> {
        let (state, failure_code) = match outcome {
            NotificationCompletion::Processed => ("processed", None),
            NotificationCompletion::Rejected { ref failure_code } => {
                validate_failure_code(failure_code)?;
                ("rejected", Some(failure_code.as_str()))
            }
        };
        let result = sqlx::query(
            "UPDATE registry_notification_inbox
             SET state = $3, claim_token = NULL, lease_expires_at = NULL,
                 failure_code = $4, processed_at = now()
             WHERE id = $1 AND claim_token = $2 AND state = 'claimed'",
        )
        .bind(id)
        .bind(claim_token)
        .bind(state)
        .bind(failure_code)
        .execute(&self.pool)
        .await
        .map_err(storage)?;
        (result.rows_affected() == 1)
            .then_some(())
            .ok_or(RegistryStoreError::Conflict)
    }
}
