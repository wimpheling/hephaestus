//! `PostgreSQL` durability adapter for the forge-owned OCI registry control plane.
//!
//! The adapter persists ownership and approval decisions only. OCI content is
//! always read from Zot by the verifier before [`PgRegistryStore::record_verified`].

use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use identity_domain::AuthenticatedIdentity;
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformDescriptor,
    PlatformImageKey, PolicyVersion, PublicationIntent, PublicationIntentId,
    PublicationLifecycleError, PublicationState, RegistryAuthority, RegistryNamespace,
    RegistryNotificationBacklog, RegistryOperationalMetrics, RegistryOwner,
    RegistryRetentionSnapshot, RegistryValueError, Sha256Digest, SupplyChainEvidence,
    SupplyChainPolicy, SupplyChainReferrer, SupplyChainReferrerKind, VerifiedPublication,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use std::time::Duration;
use time::OffsetDateTime;
use uuid::Uuid;

/// `PostgreSQL` registry-control-plane storage.
#[derive(Clone)]
pub struct PgRegistryStore {
    pool: PgPool,
}

/// Connects the standalone trusted release composition to the registry store.
///
/// # Errors
///
/// Returns an opaque storage error when `PostgreSQL` is unavailable.
pub async fn connect(database_url: &str) -> Result<PgRegistryStore, RegistryStoreError> {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await
        .map_err(storage)?;
    Ok(PgRegistryStore::new(pool))
}

// The public methods have distinct state-specific failure modes, all captured
// by the stable `RegistryStoreError` contract below.
#[allow(clippy::missing_errors_doc)]
impl PgRegistryStore {
    /// Creates the adapter with a pool authenticated as `hephaestus_worker`
    /// for mutation paths, or `hephaestus_app` for read-only paths.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Authorizes an authenticated human to pull from one live project-owned
    /// namespace and records the allow/deny decision in the existing audit
    /// journal. Platform namespaces and unknown paths are denied here; trusted
    /// publisher/runtime workload grants use their separate internal boundary.
    pub async fn authorize_user_pull(
        &self,
        identity: &AuthenticatedIdentity,
        namespace: &RegistryNamespace,
    ) -> Result<bool, RegistryStoreError> {
        let mut transaction = authz_postgres::begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let project_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT project_id FROM registry_namespaces
             WHERE repository_path = $1 AND project_id IS NOT NULL",
        )
        .bind(namespace.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let Some(project_id) = project_id else {
            transaction.commit().await.map_err(storage)?;
            return Ok(false);
        };
        let object = ObjectRef::new(ObjectType::Project, project_id);
        let decision = authz_postgres::PostgresMelangeAuthorizer
            .check(
                &mut transaction,
                Subject::User(identity.user_id),
                Permission::CanRead,
                object,
            )
            .await
            .map_err(|error| RegistryStoreError::Storage(Box::new(error)))?;
        authz_postgres::audit_decision(
            &mut transaction,
            identity.user_id,
            Permission::CanRead,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(decision.is_allowed())
    }

    /// Creates an idempotent durable namespace claim and publication intent.
    ///
    /// Repeating the exact same intent returns the existing immutable row.
    pub async fn create_intent(
        &self,
        intent: &PublicationIntent,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let namespace_id = ensure_namespace(&mut transaction, intent.claim()).await?;
        let expected = intent.expected_manifest();
        let (owner_kind, platform_image_key, owner_id, project_id) =
            owner_fields(intent.claim().owner());
        let inserted = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO registry_publications (
                id, namespace_id, owner_kind, platform_image_key, owner_id, project_id,
                registry_authority, expected_digest,
                expected_media_type, expected_size, policy_version, signature_required
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (namespace_id, registry_authority, expected_digest, policy_version)
             DO NOTHING
             RETURNING id",
        )
        .bind(intent.id().as_uuid())
        .bind(namespace_id)
        .bind(owner_kind)
        .bind(platform_image_key)
        .bind(owner_id)
        .bind(project_id)
        .bind(intent.reference().authority().as_str())
        .bind(intent.reference().digest().as_str())
        .bind(expected.media_type().as_str())
        .bind(i64::try_from(expected.size()).map_err(|_| RegistryStoreError::Conflict)?)
        .bind(intent.policy_version().as_str())
        .bind(intent.supply_chain_policy().signature_required())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let id = match inserted {
            Some(id) => id,
            None => sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM registry_publications
                 WHERE namespace_id = $1 AND registry_authority = $2
                   AND expected_digest = $3 AND policy_version = $4 FOR UPDATE",
            )
            .bind(namespace_id)
            .bind(intent.reference().authority().as_str())
            .bind(intent.reference().digest().as_str())
            .bind(intent.policy_version().as_str())
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?,
        };
        let stored = load_intent(&mut transaction, id, false).await?;
        if !same_intent_identity(&stored, intent) {
            return Err(RegistryStoreError::Conflict);
        }
        transaction.commit().await.map_err(storage)?;
        Ok(stored)
    }

    /// Loads one registry publication intent by stable identity.
    pub async fn load(
        &self,
        id: PublicationIntentId,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let intent = load_intent(&mut transaction, id.as_uuid(), false).await?;
        transaction.commit().await.map_err(storage)?;
        Ok(intent)
    }

    /// Lists durable publication intents owned by a project.
    pub async fn list_for_project(
        &self,
        project_id: forge_domain::ProjectId,
    ) -> Result<Vec<PublicationIntent>, RegistryStoreError> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT publication.id FROM registry_publications publication
             JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
             WHERE namespace.project_id = $1 ORDER BY publication.created_at, publication.id",
        )
        .bind(project_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        self.load_ids(ids).await
    }

    /// Lists durable publication intents for one exact registry namespace in
    /// creation order. This is the bounded reconciliation read for a Zot
    /// repository observation.
    pub async fn list_for_namespace(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<Vec<PublicationIntent>, RegistryStoreError> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT publication.id FROM registry_publications publication
             JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
             WHERE namespace.repository_path = $1
             ORDER BY publication.created_at, publication.id",
        )
        .bind(namespace.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        self.load_ids(ids).await
    }

    /// Lists every durable publication intent in a deterministic namespace
    /// order for scheduled full reconciliation.
    pub async fn list_all(&self) -> Result<Vec<PublicationIntent>, RegistryStoreError> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT publication.id FROM registry_publications publication
             JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
             ORDER BY namespace.repository_path, publication.created_at, publication.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        self.load_ids(ids).await
    }

    /// Lists forge-owned platform publication intents.
    pub async fn list_platform(&self) -> Result<Vec<PublicationIntent>, RegistryStoreError> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT publication.id FROM registry_publications publication
             JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
             WHERE namespace.project_id IS NULL ORDER BY publication.created_at, publication.id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        self.load_ids(ids).await
    }

    /// Loads the read-only durable input for a provider-neutral retention
    /// report. OCI inventory is deliberately supplied by a separate bounded
    /// provider adapter or operator document; this method never contacts Zot.
    pub async fn retention_snapshot(
        &self,
    ) -> Result<RegistryRetentionSnapshot, RegistryStoreError> {
        let intents = self.list_all().await?;
        let publications = sqlx::query_as::<_, PublicationMetricsRow>(
            "SELECT
                count(*) FILTER (WHERE state = 'pending') AS pending,
                count(*) FILTER (WHERE state = 'publishing') AS publishing,
                count(*) FILTER (WHERE state = 'verified') AS verified,
                count(*) FILTER (WHERE state = 'approved') AS approved,
                count(*) FILTER (WHERE state = 'retired') AS retired,
                count(*) FILTER (WHERE state = 'missing') AS missing
             FROM registry_publications",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        let notifications = sqlx::query_as::<_, NotificationMetricsRow>(
            "SELECT
                count(*) FILTER (WHERE state = 'pending') AS pending,
                count(*) FILTER (WHERE state = 'claimed') AS claimed,
                count(*) FILTER (
                    WHERE state = 'claimed' AND lease_expires_at <= now()
                ) AS expired_claims,
                count(*) FILTER (WHERE state = 'processed') AS processed,
                count(*) FILTER (WHERE state = 'rejected') AS rejected
             FROM registry_notification_inbox",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        Ok(RegistryRetentionSnapshot::new(
            intents,
            RegistryOperationalMetrics {
                pending_publications: count_to_u64(publications.pending)?,
                publishing_publications: count_to_u64(publications.publishing)?,
                verified_publications: count_to_u64(publications.verified)?,
                approved_publications: count_to_u64(publications.approved)?,
                retired_publications: count_to_u64(publications.retired)?,
                missing_publications: count_to_u64(publications.missing)?,
                notification_backlog: RegistryNotificationBacklog {
                    pending: count_to_u64(notifications.pending)?,
                    claimed: count_to_u64(notifications.claimed)?,
                    expired_claims: count_to_u64(notifications.expired_claims)?,
                    processed: count_to_u64(notifications.processed)?,
                    rejected: count_to_u64(notifications.rejected)?,
                },
            },
        ))
    }

    async fn load_ids(&self, ids: Vec<Uuid>) -> Result<Vec<PublicationIntent>, RegistryStoreError> {
        let mut intents = Vec::with_capacity(ids.len());
        for id in ids {
            intents.push(self.load(PublicationIntentId::from_uuid(id)).await?);
        }
        Ok(intents)
    }

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

/// A new authenticated Zot notification after transport validation.
#[derive(Debug, Clone)]
pub struct NewRegistryNotification {
    /// Stable upstream event key.
    pub event_key: String,
    /// Canonical repository path observed by Zot.
    pub repository_path: String,
    /// Distribution operation observed by Zot.
    pub action: RegistryNotificationAction,
    /// Optional target descriptor, when the notification includes one.
    pub target: Option<RegistryNotificationTarget>,
    /// Upstream occurrence timestamp.
    pub occurred_at: OffsetDateTime,
    /// SHA-256 of the exact transport body, never the retained body itself.
    pub payload_sha256: [u8; 32],
}

/// Digest and media type observed in a Zot event. Descriptor size is resolved
/// authoritatively from Zot during reconciliation, not trusted from callbacks.
#[derive(Debug, Clone)]
pub struct RegistryNotificationTarget {
    /// Observed manifest digest.
    pub digest: Sha256Digest,
    /// Observed manifest media type.
    pub media_type: OciMediaType,
}

impl NewRegistryNotification {
    fn validate(&self) -> Result<(), RegistryStoreError> {
        ((1..=200).contains(&self.event_key.len())
            && self.event_key == self.event_key.trim()
            && !self.event_key.bytes().any(|byte| byte.is_ascii_control())
            && valid_observed_repository_path(&self.repository_path))
        .then_some(())
        .ok_or(RegistryStoreError::InvalidNotification)
    }
}

/// Bounded Zot notification action retained by the durable inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryNotificationAction {
    /// Manifest or artifact push observation.
    Push,
    /// Pull observation.
    Pull,
    /// Delete observation.
    Delete,
}

impl RegistryNotificationAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
            Self::Delete => "delete",
        }
    }

    fn parse(value: &str) -> Result<Self, RegistryStoreError> {
        match value {
            "push" => Ok(Self::Push),
            "pull" => Ok(Self::Pull),
            "delete" => Ok(Self::Delete),
            _ => Err(RegistryStoreError::InvalidStoredData),
        }
    }
}

/// Durable notification receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryNotificationReceipt {
    /// Durable inbox identity.
    pub id: Uuid,
    /// Idempotency key.
    pub event_key: String,
    /// Whether the exact event was already durable.
    pub duplicate: bool,
}

/// A notification exclusively claimed by one reducer lease.
#[derive(Debug, Clone)]
pub struct ClaimedRegistryNotification {
    /// Durable inbox identity.
    pub id: Uuid,
    /// Lease capability required for completion.
    pub claim_token: Uuid,
    /// Canonical namespace path.
    pub repository_path: String,
    /// Parsed forge-owned namespace, absent for bounded orphan observations.
    pub namespace: Option<RegistryNamespace>,
    /// Observed action.
    pub action: RegistryNotificationAction,
    /// Optional target descriptor.
    pub target: Option<RegistryNotificationTarget>,
    /// Upstream event timestamp.
    pub occurred_at: OffsetDateTime,
}

/// Terminal reduction result for an inbox observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationCompletion {
    /// The observation has been reduced or queued for reconciliation.
    Processed,
    /// The observation is permanently invalid, with a bounded safe code.
    Rejected {
        /// Stable non-sensitive rejection code.
        failure_code: String,
    },
}

/// Registry adapter failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryStoreError {
    /// The caller supplied data that violates domain validation.
    #[error("registry value is invalid: {0}")]
    InvalidValue(#[from] RegistryValueError),
    /// A state edge or verification conflicted with the durable lifecycle.
    #[error("registry lifecycle transition is invalid: {0}")]
    Lifecycle(#[source] PublicationLifecycleError),
    /// A stored row cannot be reconstructed as a strict domain value.
    #[error("registry storage contains invalid control-plane data")]
    InvalidStoredData,
    /// An operation lost an ownership, idempotency, or lease race.
    #[error("registry control-plane operation conflicted")]
    Conflict,
    /// A notification failed local bounded-field validation.
    #[error("registry notification is invalid")]
    InvalidNotification,
    /// `PostgreSQL` failed.
    #[error("registry storage failed")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Clone, Copy)]
enum Transition {
    Begin,
    Retry,
    Approve,
    Missing,
    Retire,
}

impl Transition {
    fn apply(self, intent: PublicationIntent) -> Result<PublicationIntent, RegistryStoreError> {
        match self {
            Self::Begin => intent.begin_publishing(),
            Self::Retry => intent.retry(),
            Self::Approve => intent.approve(),
            Self::Missing => intent.mark_missing(),
            Self::Retire => Ok(intent.retire()),
        }
        .map_err(RegistryStoreError::Lifecycle)
    }
}

#[derive(FromRow)]
struct PublicationRow {
    id: Uuid,
    repository_path: String,
    owner_kind: String,
    platform_image_key: Option<String>,
    owner_id: Option<Uuid>,
    project_id: Option<Uuid>,
    registry_authority: String,
    expected_digest: String,
    expected_media_type: String,
    expected_size: i64,
    policy_version: String,
    signature_required: bool,
    state: String,
    verified_at: Option<OffsetDateTime>,
    approved_at: Option<OffsetDateTime>,
}

#[derive(FromRow)]
struct PlatformRow {
    digest: String,
    size: i64,
    media_type: String,
    operating_system: String,
    architecture: String,
    variant: Option<String>,
}

#[derive(FromRow)]
struct EvidenceRow {
    kind: String,
    subject_digest: String,
    digest: String,
    size: i64,
    media_type: String,
    artifact_type: String,
}

#[derive(FromRow)]
struct PublicationMetricsRow {
    pending: i64,
    publishing: i64,
    verified: i64,
    approved: i64,
    retired: i64,
    missing: i64,
}

#[derive(FromRow)]
struct NotificationMetricsRow {
    pending: i64,
    claimed: i64,
    expired_claims: i64,
    processed: i64,
    rejected: i64,
}

#[derive(FromRow)]
struct NotificationRow {
    id: Uuid,
    event_key: String,
    repository_path: String,
    action: String,
    target_digest: Option<String>,
    target_media_type: Option<String>,
    target_size: Option<i64>,
    event_occurred_at: OffsetDateTime,
    payload_sha256: Vec<u8>,
    state: String,
    claim_token: Option<Uuid>,
    lease_expires_at: Option<OffsetDateTime>,
    failure_code: Option<String>,
    processed_at: Option<OffsetDateTime>,
}

async fn ensure_namespace(
    transaction: &mut Transaction<'_, Postgres>,
    claim: &NamespaceClaim,
) -> Result<Uuid, RegistryStoreError> {
    let (owner_kind, platform_image_key, owner_id, project_id) = owner_fields(claim.owner());
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO registry_namespaces (
            id, repository_path, owner_kind, platform_image_key, owner_id, project_id
         ) VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (repository_path) DO NOTHING RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(claim.namespace().as_str())
    .bind(owner_kind)
    .bind(platform_image_key)
    .bind(owner_id)
    .bind(project_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    let id = if let Some(id) = inserted {
        id
    } else {
        let stored = sqlx::query_as::<_, NamespaceRow>(
            "SELECT id, repository_path, owner_kind, platform_image_key, owner_id, project_id
             FROM registry_namespaces WHERE repository_path = $1 FOR UPDATE",
        )
        .bind(claim.namespace().as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        if stored.repository_path != claim.namespace().as_str()
            || stored.owner_kind != owner_kind
            || stored.platform_image_key.as_deref() != platform_image_key
            || stored.owner_id != owner_id
            || stored.project_id != project_id
        {
            return Err(RegistryStoreError::Conflict);
        }
        stored.id
    };
    Ok(id)
}

#[derive(FromRow)]
struct NamespaceRow {
    id: Uuid,
    repository_path: String,
    owner_kind: String,
    platform_image_key: Option<String>,
    owner_id: Option<Uuid>,
    project_id: Option<Uuid>,
}

async fn load_intent(
    transaction: &mut Transaction<'_, Postgres>,
    id: Uuid,
    for_update: bool,
) -> Result<PublicationIntent, RegistryStoreError> {
    let row = if for_update {
        sqlx::query_as::<_, PublicationRow>(
            "SELECT publication.id, namespace.repository_path, namespace.owner_kind,
            namespace.platform_image_key, namespace.owner_id, namespace.project_id,
            publication.registry_authority, publication.expected_digest,
            publication.expected_media_type, publication.expected_size,
            publication.policy_version, publication.signature_required, publication.state
            , publication.verified_at, publication.approved_at
         FROM registry_publications publication
         JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
         WHERE publication.id = $1 FOR UPDATE OF publication",
        )
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
    } else {
        sqlx::query_as::<_, PublicationRow>(
            "SELECT publication.id, namespace.repository_path, namespace.owner_kind,
            namespace.platform_image_key, namespace.owner_id, namespace.project_id,
            publication.registry_authority, publication.expected_digest,
            publication.expected_media_type, publication.expected_size,
            publication.policy_version, publication.signature_required, publication.state
            , publication.verified_at, publication.approved_at
         FROM registry_publications publication
         JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
         WHERE publication.id = $1",
        )
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
    }
    .ok_or(RegistryStoreError::Conflict)?;
    let platforms = sqlx::query_as::<_, PlatformRow>(
        "SELECT digest, size, media_type, operating_system, architecture, variant
         FROM registry_publication_platforms WHERE publication_id = $1 ORDER BY digest",
    )
    .bind(id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(storage)?;
    let evidence = sqlx::query_as::<_, EvidenceRow>(
        "SELECT kind, subject_digest, digest, size, media_type, artifact_type
         FROM registry_publication_evidence WHERE publication_id = $1 ORDER BY kind, digest",
    )
    .bind(id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(storage)?;
    publication_from_rows(row, platforms, evidence)
}

fn publication_from_rows(
    row: PublicationRow,
    platforms: Vec<PlatformRow>,
    evidence: Vec<EvidenceRow>,
) -> Result<PublicationIntent, RegistryStoreError> {
    let owner = match row.owner_kind.as_str() {
        "platform_image" => RegistryOwner::PlatformImage {
            image_key: PlatformImageKey::parse(
                row.platform_image_key
                    .ok_or(RegistryStoreError::InvalidStoredData)?,
            )?,
        },
        "repository_oci_image" => RegistryOwner::RepositoryOciImage {
            project_id: forge_domain::ProjectId::from_uuid(
                row.project_id
                    .ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
            image_id: builder_catalog_domain::OciImageId::from_uuid(
                row.owner_id.ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
        },
        "release_agent" => RegistryOwner::ReleaseAgent {
            project_id: forge_domain::ProjectId::from_uuid(
                row.project_id
                    .ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
            release_agent_id: runtime_types::ReleaseAgentId::from_uuid(
                row.owner_id.ok_or(RegistryStoreError::InvalidStoredData)?,
            ),
        },
        _ => return Err(RegistryStoreError::InvalidStoredData),
    };
    let claim = NamespaceClaim::new(owner);
    if claim.namespace().as_str() != row.repository_path {
        return Err(RegistryStoreError::InvalidStoredData);
    }
    let digest = Sha256Digest::parse(row.expected_digest)?;
    let reference = ImmutableManifestReference::new(
        RegistryAuthority::parse(row.registry_authority)?,
        RegistryNamespace::parse(row.repository_path)?,
        digest.clone(),
    );
    let expected = descriptor(digest, row.expected_size, row.expected_media_type)?;
    let mut intent = PublicationIntent::new(
        PublicationIntentId::from_uuid(row.id),
        claim,
        reference,
        expected.clone(),
        PolicyVersion::parse(row.policy_version)?,
        if row.signature_required {
            SupplyChainPolicy::with_signature()
        } else {
            SupplyChainPolicy::without_signature()
        },
    )?;
    let state = parse_state(&row.state)?;
    if matches!(state, PublicationState::Publishing) {
        intent = intent
            .begin_publishing()
            .map_err(RegistryStoreError::Lifecycle)?;
    }
    if matches!(
        state,
        PublicationState::Verified | PublicationState::Approved | PublicationState::Missing
    ) || (state == PublicationState::Retired && row.verified_at.is_some())
    {
        let verification = verification_from_rows(
            &reference_from_intent(&intent),
            expected,
            platforms,
            evidence,
        )?;
        intent = intent
            .record_verified(verification)
            .map_err(RegistryStoreError::Lifecycle)?;
        if matches!(
            state,
            PublicationState::Approved | PublicationState::Missing
        ) || (state == PublicationState::Retired && row.approved_at.is_some())
        {
            intent = intent.approve().map_err(RegistryStoreError::Lifecycle)?;
        }
        if state == PublicationState::Missing {
            intent = intent
                .mark_missing()
                .map_err(RegistryStoreError::Lifecycle)?;
        }
    }
    if state == PublicationState::Retired {
        intent = intent.retire();
    }
    Ok(intent)
}

fn reference_from_intent(intent: &PublicationIntent) -> ImmutableManifestReference {
    intent.reference().clone()
}

fn verification_from_rows(
    reference: &ImmutableManifestReference,
    manifest: OciDescriptor,
    platforms: Vec<PlatformRow>,
    evidence: Vec<EvidenceRow>,
) -> Result<VerifiedPublication, RegistryStoreError> {
    let platforms = platforms
        .into_iter()
        .map(|row| {
            let descriptor =
                descriptor(Sha256Digest::parse(row.digest)?, row.size, row.media_type)?;
            PlatformDescriptor::new(
                descriptor,
                row.operating_system,
                row.architecture,
                row.variant,
            )
            .map_err(RegistryStoreError::InvalidValue)
        })
        .collect::<Result<Vec<_>, RegistryStoreError>>()?;
    let referrers = evidence
        .into_iter()
        .map(|row| {
            Ok(SupplyChainReferrer::new(
                parse_referrer_kind(&row.kind)?,
                Sha256Digest::parse(row.subject_digest)?,
                descriptor(Sha256Digest::parse(row.digest)?, row.size, row.media_type)?,
                OciMediaType::parse(row.artifact_type)?,
            ))
        })
        .collect::<Result<Vec<_>, RegistryStoreError>>()?;
    let evidence = SupplyChainEvidence::new(reference.digest().clone(), referrers)?;
    Ok(VerifiedPublication::new(
        reference, manifest, platforms, evidence,
    )?)
}

async fn insert_verification(
    transaction: &mut Transaction<'_, Postgres>,
    publication_id: Uuid,
    verification: &VerifiedPublication,
) -> Result<(), RegistryStoreError> {
    for platform in verification.platforms() {
        sqlx::query(
            "INSERT INTO registry_publication_platforms (
                publication_id, digest, size, media_type, operating_system, architecture, variant
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(publication_id)
        .bind(platform.descriptor().digest().as_str())
        .bind(
            i64::try_from(platform.descriptor().size())
                .map_err(|_| RegistryStoreError::Conflict)?,
        )
        .bind(platform.descriptor().media_type().as_str())
        .bind(platform.operating_system())
        .bind(platform.architecture())
        .bind(platform.variant())
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    for referrer in verification.evidence().referrers() {
        sqlx::query(
            "INSERT INTO registry_publication_evidence (
                publication_id, kind, subject_digest, digest, size, media_type, artifact_type
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(publication_id)
        .bind(referrer_kind_text(referrer.kind()))
        .bind(referrer.subject().as_str())
        .bind(referrer.descriptor().digest().as_str())
        .bind(
            i64::try_from(referrer.descriptor().size())
                .map_err(|_| RegistryStoreError::Conflict)?,
        )
        .bind(referrer.descriptor().media_type().as_str())
        .bind(referrer.artifact_type().as_str())
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    Ok(())
}

fn owner_fields(owner: &RegistryOwner) -> (&'static str, Option<&str>, Option<Uuid>, Option<Uuid>) {
    match owner {
        RegistryOwner::PlatformImage { image_key } => {
            ("platform_image", Some(image_key.as_str()), None, None)
        }
        RegistryOwner::RepositoryOciImage {
            project_id,
            image_id,
        } => (
            "repository_oci_image",
            None,
            Some(image_id.as_uuid()),
            Some(project_id.as_uuid()),
        ),
        RegistryOwner::ReleaseAgent {
            project_id,
            release_agent_id,
        } => (
            "release_agent",
            None,
            Some(release_agent_id.as_uuid()),
            Some(project_id.as_uuid()),
        ),
    }
}

fn same_intent_identity(left: &PublicationIntent, right: &PublicationIntent) -> bool {
    // The caller creates a fresh ID for each submission. When the unique
    // durable identity already exists, `create_intent` must return that row
    // so an interrupted publication can be retried; the generated request ID
    // is deliberately not part of the idempotency comparison.
    left.claim() == right.claim()
        && left.reference() == right.reference()
        && left.expected_manifest() == right.expected_manifest()
        && left.policy_version() == right.policy_version()
        && left.supply_chain_policy() == right.supply_chain_policy()
}

fn descriptor(
    digest: Sha256Digest,
    size: i64,
    media_type: String,
) -> Result<OciDescriptor, RegistryStoreError> {
    let size = u64::try_from(size).map_err(|_| RegistryStoreError::InvalidStoredData)?;
    Ok(OciDescriptor::new(
        digest,
        size,
        OciMediaType::parse(media_type)?,
    )?)
}

fn parse_referrer_kind(value: &str) -> Result<SupplyChainReferrerKind, RegistryStoreError> {
    match value {
        "sbom" => Ok(SupplyChainReferrerKind::Sbom),
        "provenance" => Ok(SupplyChainReferrerKind::Provenance),
        "scan" => Ok(SupplyChainReferrerKind::Scan),
        "signature" => Ok(SupplyChainReferrerKind::Signature),
        _ => Err(RegistryStoreError::InvalidStoredData),
    }
}

const fn referrer_kind_text(value: SupplyChainReferrerKind) -> &'static str {
    match value {
        SupplyChainReferrerKind::Sbom => "sbom",
        SupplyChainReferrerKind::Provenance => "provenance",
        SupplyChainReferrerKind::Scan => "scan",
        SupplyChainReferrerKind::Signature => "signature",
    }
}

fn parse_state(value: &str) -> Result<PublicationState, RegistryStoreError> {
    match value {
        "pending" => Ok(PublicationState::Pending),
        "publishing" => Ok(PublicationState::Publishing),
        "verified" => Ok(PublicationState::Verified),
        "approved" => Ok(PublicationState::Approved),
        "retired" => Ok(PublicationState::Retired),
        "missing" => Ok(PublicationState::Missing),
        _ => Err(RegistryStoreError::InvalidStoredData),
    }
}

const fn state_text(value: PublicationState) -> &'static str {
    match value {
        PublicationState::Pending => "pending",
        PublicationState::Publishing => "publishing",
        PublicationState::Verified => "verified",
        PublicationState::Approved => "approved",
        PublicationState::Retired => "retired",
        PublicationState::Missing => "missing",
    }
}

impl NotificationRow {
    fn try_into_receipt(self) -> Result<RegistryNotificationReceipt, RegistryStoreError> {
        let _ = self.try_into_claimed_parts()?;
        Ok(RegistryNotificationReceipt {
            id: self.id,
            event_key: self.event_key,
            duplicate: false,
        })
    }

    fn try_into_claimed(self) -> Result<ClaimedRegistryNotification, RegistryStoreError> {
        let (repository_path, namespace, action, target) = self.try_into_claimed_parts()?;
        Ok(ClaimedRegistryNotification {
            id: self.id,
            claim_token: self
                .claim_token
                .ok_or(RegistryStoreError::InvalidStoredData)?,
            repository_path,
            namespace,
            action,
            target,
            occurred_at: self.event_occurred_at,
        })
    }

    fn try_into_claimed_parts(
        &self,
    ) -> Result<
        (
            String,
            Option<RegistryNamespace>,
            RegistryNotificationAction,
            Option<RegistryNotificationTarget>,
        ),
        RegistryStoreError,
    > {
        if self.payload_sha256.len() != 32
            || (self.state == "claimed" && self.lease_expires_at.is_none())
            || ((self.state == "processed" || self.state == "rejected")
                && self.processed_at.is_none())
            || (self.state == "rejected" && self.failure_code.is_none())
        {
            return Err(RegistryStoreError::InvalidStoredData);
        }
        let target = match (
            &self.target_digest,
            &self.target_media_type,
            self.target_size,
        ) {
            (None, None, None) => None,
            (Some(digest), Some(media_type), None) => Some(RegistryNotificationTarget {
                digest: Sha256Digest::parse(digest.clone())?,
                media_type: OciMediaType::parse(media_type.clone())?,
            }),
            (Some(digest), Some(media_type), Some(size)) => {
                let descriptor = descriptor(
                    Sha256Digest::parse(digest.clone())?,
                    size,
                    media_type.clone(),
                )?;
                Some(RegistryNotificationTarget {
                    digest: descriptor.digest().clone(),
                    media_type: descriptor.media_type().clone(),
                })
            }
            _ => return Err(RegistryStoreError::InvalidStoredData),
        };
        if !valid_observed_repository_path(&self.repository_path) {
            return Err(RegistryStoreError::InvalidStoredData);
        }
        Ok((
            self.repository_path.clone(),
            RegistryNamespace::parse(self.repository_path.clone()).ok(),
            RegistryNotificationAction::parse(&self.action)?,
            target,
        ))
    }
}

fn valid_observed_repository_path(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value.split('/').all(|component| {
            !component.is_empty()
                && component.len() <= 128
                && component.bytes().enumerate().all(|(index, byte)| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
                })
                && !component.ends_with(['.', '_', '-'])
        })
}

fn validate_failure_code(value: &str) -> Result<(), RegistryStoreError> {
    (!value.is_empty()
        && value.len() <= 64
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (byte == b'_' && index > 0)
        }))
    .then_some(())
    .ok_or(RegistryStoreError::InvalidNotification)
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> RegistryStoreError {
    RegistryStoreError::Storage(Box::new(error))
}

fn count_to_u64(value: i64) -> Result<u64, RegistryStoreError> {
    u64::try_from(value).map_err(|_| RegistryStoreError::InvalidStoredData)
}
