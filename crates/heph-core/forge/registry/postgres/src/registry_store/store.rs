use super::prelude::*;
use super::{
    notification::RegistryStoreError,
    parsing::{count_to_u64, owner_fields, same_intent_identity, storage},
    rows::{NotificationMetricsRow, PublicationMetricsRow, ensure_namespace, load_intent},
};
/// `PostgreSQL` registry-control-plane storage.
#[derive(Clone)]
pub struct PgRegistryStore {
    pub(super) pool: PgPool,
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
}
