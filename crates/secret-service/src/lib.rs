//! Transactional secret creation, rotation, grant, import, revocation, and
//! cryptographic purge.
//!
//! Command payloads containing [`SecretValue`] are never serialized into the
//! inbox or outbox. Only encrypted envelopes and opaque identifiers cross a
//! durable boundary.

use async_nats::{HeaderMap, jetstream};
use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, Authorizer, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{
    AUTHORIZATION_MODEL_VERSION, audit_decision, begin_actor_transaction, begin_runtime_transaction,
};
use forge_domain::{CommitSha, GitRef, ProjectId};
use identity_domain::{AuthenticatedIdentity, OrganizationId};
use release_domain::{AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretLeaseId, SecretName, SecretOwner,
    SecretRuntimeSessionId, SecretSlotKey, SecretTarget, SecretUsePolicy, SecretValue,
    SecretVersionId,
};
use secret_store::{
    EncryptedSecretVersion, EncryptedStore, KeyProvider, SecretStoreError, VersionContext,
};
use serde_json::{Value, json};
use sha2::Digest;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

const SECRET_EVENT_STREAM: &str = "HEPHAESTUS_SECRET_EVENTS";

/// Creates the durable stream for safe secret lifecycle and reconciliation
/// events.
///
/// # Errors
///
/// Returns an error when `JetStream` rejects topology creation.
pub async fn ensure_secret_jetstream_topology(
    context: &jetstream::Context,
) -> Result<(), SecretOutboxPublishError> {
    use jetstream::stream::{Config, RetentionPolicy, StorageType};

    context
        .get_or_create_stream(Config {
            name: SECRET_EVENT_STREAM.to_owned(),
            subjects: vec![String::from("hephaestus.secret.>")],
            retention: RetentionPolicy::Limits,
            storage: StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(|error| SecretOutboxPublishError::JetStream(error.to_string()))?;
    Ok(())
}

/// Publishes secret-owned transactional outbox records to `JetStream`.
#[derive(Clone)]
pub struct SecretOutboxPublisher {
    context: jetstream::Context,
    pool: PgPool,
}

impl SecretOutboxPublisher {
    /// Creates a publisher for secret-owned records.
    #[must_use]
    pub const fn new(context: jetstream::Context, pool: PgPool) -> Self {
        Self { context, pool }
    }

    /// Publishes and marks up to `limit` pending records.
    ///
    /// `Nats-Msg-Id` is the durable outbox identity, so an acknowledgement
    /// loss followed by retry cannot append a duplicate stream message.
    ///
    /// # Errors
    ///
    /// Returns after recording the first database or publication failure.
    pub async fn publish_pending(&self, limit: i64) -> Result<usize, SecretOutboxPublishError> {
        let rows = sqlx::query_as::<_, SecretOutboxRow>(
            "SELECT id, subject, payload FROM outbox
             WHERE published_at IS NULL AND aggregate_type = 'secret'
             ORDER BY occurred_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let count = rows.len();
        for row in rows {
            let payload = serde_json::to_vec(&row.payload)?;
            let mut headers = HeaderMap::new();
            headers.insert("Nats-Msg-Id", row.id.to_string());
            let publication = self
                .context
                .publish_with_headers(row.subject, headers, payload.into())
                .await;
            let result = match publication {
                Ok(acknowledgement) => acknowledgement.await.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            match result {
                Ok(_) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET published_at = now(), attempts = attempts + 1,
                             last_error = NULL
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .execute(&self.pool)
                    .await?;
                }
                Err(error) => {
                    sqlx::query(
                        "UPDATE outbox
                         SET attempts = attempts + 1, last_error = $2
                         WHERE id = $1",
                    )
                    .bind(row.id)
                    .bind(&error)
                    .execute(&self.pool)
                    .await?;
                    return Err(SecretOutboxPublishError::JetStream(error));
                }
            }
        }
        Ok(count)
    }
}

#[derive(sqlx::FromRow)]
struct SecretOutboxRow {
    id: Uuid,
    subject: String,
    payload: Value,
}

/// Secret outbox database, serialization, or publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretOutboxPublishError {
    /// `PostgreSQL` access failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// Command serialization failed.
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    /// `JetStream` rejected publication.
    #[error("JetStream publication failed: {0}")]
    JetStream(String),
}

/// Initial immutable secret version command. Plaintext has no formatting or
/// serialization implementation that can expose its contents.
pub struct CreateSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable caller-selected secret identity.
    pub secret_id: SecretId,
    /// Stable first version identity.
    pub version_id: SecretVersionId,
    /// Exact owner.
    pub owner: SecretOwner,
    /// Owner-scoped name.
    pub name: SecretName,
    /// Owner ceiling.
    pub allowed_delivery_modes: Vec<DeliveryMode>,
    /// Initial plaintext accepted only at this boundary.
    pub value: SecretValue,
}

/// Rotate to one exact new immutable version.
pub struct RotateSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Secret being rotated.
    pub secret_id: SecretId,
    /// Compare-and-swap expected active version.
    pub expected_active_version_id: SecretVersionId,
    /// Stable new version identity.
    pub new_version_id: SecretVersionId,
    /// Replacement plaintext.
    pub value: SecretValue,
}

/// Offer a bounded source grant to one exact target.
#[derive(Debug, Clone)]
pub struct GrantSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable grant identity.
    pub grant_id: SecretGrantId,
    /// Source secret.
    pub secret_id: SecretId,
    /// Exact target.
    pub target: SecretTarget,
    /// Bounded delivery policy.
    pub policy: SecretUsePolicy,
    /// Optional expiration.
    pub expires_at: Option<OffsetDateTime>,
}

/// Accept one exact active source grant under a local alias.
#[derive(Debug, Clone)]
pub struct AcceptSecretImport {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable import identity.
    pub import_id: SecretImportId,
    /// Source grant.
    pub grant_id: SecretGrantId,
    /// Exact accepting target.
    pub target: SecretTarget,
    /// Target-local alias.
    pub alias: SecretAlias,
}

/// Atomically creates a source grant and accepts its target-side import.
///
/// This convenience command is intentionally available only when one actor
/// independently holds both source grant-management and target import-
/// acceptance authority.
#[derive(Debug, Clone)]
pub struct GrantAndAcceptSecretImport {
    /// Deterministic identity for the compound command.
    pub command_key: SecretCommandKey,
    /// Stable source grant identity.
    pub grant_id: SecretGrantId,
    /// Source secret.
    pub secret_id: SecretId,
    /// Exact grant and import target.
    pub target: SecretTarget,
    /// Bounded use policy.
    pub policy: SecretUsePolicy,
    /// Optional grant expiration.
    pub expires_at: Option<OffsetDateTime>,
    /// Stable accepted import identity.
    pub import_id: SecretImportId,
    /// Target-local opaque alias.
    pub alias: SecretAlias,
}

/// Creates a new immutable instance revision whose declared symbolic slot is
/// bound to one eligible opaque import.
#[derive(Debug, Clone)]
pub struct BindSecret {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Stable new binding identity for the selected slot.
    pub binding_id: AgentSecretBindingId,
    /// Parent instance.
    pub instance_id: AgentInstanceId,
    /// Compare-and-swap active source revision.
    pub expected_revision_id: AgentInstanceRevisionId,
    /// Stable new immutable revision.
    pub new_revision_id: AgentInstanceRevisionId,
    /// Opaque target import.
    pub import_id: SecretImportId,
    /// Declared release slot.
    pub slot: SecretSlotKey,
    /// Exact delivery mode.
    pub mode: DeliveryMode,
    /// Selected declared phases.
    pub phases: Vec<ExecutionPhase>,
    /// Exact selected instance attachments.
    pub attachment_ids: Vec<Uuid>,
    /// Selected broker destination ceiling.
    pub destinations: Vec<String>,
}

/// Resolves one exact runnable instance revision immediately before dispatch.
#[derive(Debug, Clone)]
pub struct ResolveRunSecrets {
    /// Deterministic idempotency identity.
    pub command_key: SecretCommandKey,
    /// Caller-selected runtime session identity.
    pub session_id: SecretRuntimeSessionId,
    /// Existing pre-start run receiving the authority.
    pub run_id: RunId,
    /// Exact project-owned instance.
    pub instance_id: AgentInstanceId,
    /// Exact active immutable revision.
    pub instance_revision_id: AgentInstanceRevisionId,
    /// Exact enabled target attachment for a normal run.
    pub attachment_id: Option<AgentAttachmentId>,
    /// Exact target ref for a normal run.
    pub target_ref: Option<GitRef>,
    /// Exact target commit for a normal run.
    pub target_commit: Option<CommitSha>,
    /// Normal run or update hook.
    pub phase: ExecutionPhase,
    /// Short runtime-authority expiry.
    pub expires_at: OffsetDateTime,
}

/// One non-sensitive exact lease included in a runtime session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedSecretLease {
    /// Stable lease identity.
    pub lease_id: SecretLeaseId,
    /// Symbolic release slot.
    pub slot: SecretSlotKey,
    /// Authorized delivery mode.
    pub mode: DeliveryMode,
    /// Exact immutable version pinned for this run.
    pub version_id: SecretVersionId,
}

/// Fresh bearer authority returned only by the successful dispatch call.
pub struct RuntimeSecretAuthority {
    /// Stable session identity persisted with its token hash.
    pub session_id: SecretRuntimeSessionId,
    /// Fresh opaque credential. Formatting and serialization are redacted.
    pub credential: secret_domain::OpaqueRuntimeCredential,
    /// Exact value-free lease metadata.
    pub leases: Vec<IssuedSecretLease>,
}

/// One authenticated raw value handed only to the ephemeral mount builder.
pub struct ResolvedRawSecret {
    /// Exact lease authorizing the materialization.
    pub lease_id: SecretLeaseId,
    /// Stable symbolic file name.
    pub slot: SecretSlotKey,
    /// Short-lived plaintext with redacted formatting and serialization.
    pub value: SecretValue,
}

/// Bounded semantic broker request from one exact runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerRequest {
    /// Claimed run identity, matched against the opaque credential.
    pub run_id: RunId,
    /// Symbolic released slot.
    pub slot: SecretSlotKey,
    /// Exact allowlisted destination name.
    pub destination: String,
    /// Adapter-defined semantic operation, never an arbitrary URL.
    pub operation: String,
    /// Bounded application body.
    pub body: Vec<u8>,
}

/// Sanitized broker response without upstream headers or credential material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerResponse {
    /// Provider-neutral result status.
    pub status: BrokerStatus,
    /// Bounded adapter-sanitized response body.
    pub body: Vec<u8>,
}

/// Provider-neutral broker outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerStatus {
    /// Semantic operation completed.
    Succeeded,
    /// Upstream rejected the semantic operation.
    Rejected,
    /// A retry may succeed without changing the request.
    Retryable,
}

/// Host-only application adapter. Implementations own DNS, transport,
/// redirect, metadata-endpoint, and response sanitization policy.
#[async_trait]
pub trait BrokerAdapter: Send + Sync {
    /// Applies the exact credential to one semantic allowlisted operation.
    ///
    /// Implementations must not return upstream authorization headers,
    /// credential-bearing redirects, or raw provider error bodies.
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError>;
}

/// Sanitized adapter failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BrokerAdapterError {
    /// The semantic operation is not allowed.
    #[error("broker operation is rejected")]
    Rejected,
    /// The provider may be retried.
    #[error("broker provider is temporarily unavailable")]
    Retryable,
}

/// Identifiers returned after creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedSecret {
    /// Owned secret.
    pub secret_id: SecretId,
    /// Active immutable version.
    pub version_id: SecretVersionId,
}

/// Transactional encrypted secret command service.
#[derive(Clone)]
pub struct SecretService<K> {
    pool: PgPool,
    encrypted_store: EncryptedStore<K>,
    authorizer: Arc<dyn Authorizer>,
}

/// Agent-facing runtime service with a non-bypass authorization pool and a
/// distinct narrow worker pool for exact ciphertext resolution.
#[derive(Clone)]
pub struct SecretRuntimeService<K> {
    authorization_pool: PgPool,
    resolver_pool: PgPool,
    encrypted_store: EncryptedStore<K>,
    authorizer: Arc<dyn Authorizer>,
}

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Creates a service with explicit encryption and authorization providers.
    #[must_use]
    pub fn new(
        pool: PgPool,
        encrypted_store: EncryptedStore<K>,
        authorizer: Arc<dyn Authorizer>,
    ) -> Self {
        Self {
            pool,
            encrypted_store,
            authorizer,
        }
    }

    /// Creates a secret and its first encrypted version atomically.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for denial, invalid owner/mode, encryption
    /// failure, idempotency conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            secret_id = %command.secret_id,
            secret_version_id = %command.version_id
        )
    )]
    pub async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateSecret,
    ) -> Result<CreatedSecret, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let (owner_type, owner_id, owner_organization_id, project_id, organization_id) =
            resolve_owner(&mut tx, command.owner).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanWriteSecretValue,
            ObjectRef::new(owner_type, owner_id),
        )
        .await?;
        if let Some((aggregate_id, secondary_id)) =
            existing_command(&mut tx, command.command_key, "create").await?
        {
            tx.commit().await?;
            return Ok(CreatedSecret {
                secret_id: SecretId::from_uuid(aggregate_id),
                version_id: SecretVersionId::from_uuid(
                    secondary_id.ok_or(SecretServiceError::CorruptIdempotencyRecord)?,
                ),
            });
        }
        let modes = normalized_modes(&command.allowed_delivery_modes)?;
        let context = VersionContext {
            owner: command.owner,
            secret_id: command.secret_id,
            version_id: command.version_id,
            sequence: 1,
            media_type: String::from("application/octet-stream"),
        };
        let encrypted = self.encrypted_store.seal(&context, &command.value)?;
        sqlx::query(
            "INSERT INTO secrets
             (id, owner_organization_id, organization_id, project_id, name,
              status, allowed_delivery_modes, active_version_id, created_by)
             VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, $8)",
        )
        .bind(command.secret_id.as_uuid())
        .bind(owner_organization_id)
        .bind(organization_id)
        .bind(project_id)
        .bind(command.name.as_str())
        .bind(&modes)
        .bind(command.version_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        insert_encrypted_version(
            &mut tx,
            command.secret_id,
            1,
            &encrypted,
            identity.user_id.as_uuid(),
        )
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "create",
            command.secret_id.as_uuid(),
            Some(command.version_id.as_uuid()),
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "write_value",
            "secret.write_value",
            Some(command.secret_id),
            Some(command.version_id),
            None,
            None,
            "created",
        )
        .await?;
        append_event(
            &mut tx,
            command.secret_id.as_uuid(),
            "hephaestus.secret.created.v1",
            "secret.created.v1",
            json!({
                "schema_version": 1,
                "secret_id": command.secret_id,
                "secret_version_id": command.version_id,
                "owner_type": owner_type.as_str(),
                "owner_id": owner_id,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(CreatedSecret {
            secret_id: command.secret_id,
            version_id: command.version_id,
        })
    }

    /// Rotates and compare-and-swap activates a new immutable version.
    ///
    /// # Errors
    ///
    /// Fails closed for denial, stale active versions, revoked state,
    /// encryption failure, or database errors.
    // Keeping encryption, version insert, and active-version CAS together
    // makes the transaction's security boundary directly auditable.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            secret_id = %command.secret_id,
            secret_version_id = %command.new_version_id
        )
    )]
    pub async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotateSecret,
    ) -> Result<SecretVersionId, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::Rotate,
            ObjectRef::new(ObjectType::Secret, command.secret_id.as_uuid()),
        )
        .await?;
        if let Some((_aggregate_id, secondary_id)) =
            existing_command(&mut tx, command.command_key, "rotate").await?
        {
            tx.commit().await?;
            return Ok(SecretVersionId::from_uuid(
                secondary_id.ok_or(SecretServiceError::CorruptIdempotencyRecord)?,
            ));
        }
        let row: SecretRotationRow = sqlx::query_as(
            "SELECT owner_organization_id, organization_id, project_id,
                    active_version_id,
                    COALESCE((SELECT max(sequence) FROM secret_versions
                              WHERE secret_id = secrets.id), 0) AS sequence
             FROM secrets WHERE id = $1 AND status = 'active'
             FOR UPDATE",
        )
        .bind(command.secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        if row.active_version_id != Some(command.expected_active_version_id.as_uuid()) {
            return Err(SecretServiceError::StaleActiveVersion);
        }
        let sequence = u64::try_from(row.sequence)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(SecretServiceError::VersionSequenceExhausted)?;
        let owner = row.owner()?;
        let context = VersionContext {
            owner,
            secret_id: command.secret_id,
            version_id: command.new_version_id,
            sequence,
            media_type: String::from("application/octet-stream"),
        };
        let encrypted = self.encrypted_store.seal(&context, &command.value)?;
        insert_encrypted_version(
            &mut tx,
            command.secret_id,
            sequence,
            &encrypted,
            identity.user_id.as_uuid(),
        )
        .await?;
        let changed = sqlx::query(
            "UPDATE secrets SET active_version_id = $2, updated_at = now()
             WHERE id = $1 AND status = 'active' AND active_version_id = $3",
        )
        .bind(command.secret_id.as_uuid())
        .bind(command.new_version_id.as_uuid())
        .bind(command.expected_active_version_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(SecretServiceError::StaleActiveVersion);
        }
        record_command(
            &mut tx,
            command.command_key,
            "rotate",
            command.secret_id.as_uuid(),
            Some(command.new_version_id.as_uuid()),
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            row.owner_organization_id,
            "rotate",
            "secret.rotate",
            Some(command.secret_id),
            Some(command.new_version_id),
            None,
            None,
            "activated",
        )
        .await?;
        append_event(
            &mut tx,
            command.secret_id.as_uuid(),
            "hephaestus.secret.rotated.v1",
            "secret.rotated.v1",
            json!({
                "schema_version": 1,
                "secret_id": command.secret_id,
                "previous_version_id": command.expected_active_version_id,
                "active_version_id": command.new_version_id,
                "sequence": sequence,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.new_version_id)
    }

    /// Creates one exact, bounded source-side grant.
    ///
    /// # Errors
    ///
    /// Fails for denial, cross-organization target, invalid policy, inactive
    /// secret, idempotency conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            secret_id = %command.secret_id,
            grant_id = %command.grant_id
        )
    )]
    pub async fn grant(
        &self,
        identity: &AuthenticatedIdentity,
        mut command: GrantSecret,
    ) -> Result<SecretGrantId, SecretServiceError> {
        command.policy = command.policy.normalized()?;
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::ManageGrants,
            ObjectRef::new(ObjectType::Secret, command.secret_id.as_uuid()),
        )
        .await?;
        if let Some((aggregate_id, _)) =
            existing_command(&mut tx, command.command_key, "grant").await?
        {
            tx.commit().await?;
            return Ok(SecretGrantId::from_uuid(aggregate_id));
        }
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "SELECT owner_organization_id FROM secrets
             WHERE id = $1 AND status = 'active'",
        )
        .bind(command.secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        let target = resolve_target(&mut tx, command.target).await?;
        if target.organization_id != owner_organization_id {
            return Err(SecretServiceError::CrossOrganization);
        }
        let modes = command
            .policy
            .delivery_modes
            .iter()
            .map(|mode| mode_name(*mode))
            .collect::<Vec<_>>();
        let phases = command
            .policy
            .phases
            .iter()
            .map(|phase| phase_name(*phase))
            .collect::<Vec<_>>();
        sqlx::query(
            "INSERT INTO secret_grants
             (id, secret_id, owner_organization_id, target_kind, target_id,
              target_project_id, delivery_modes, phases, destinations, status,
              expires_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10, $11)",
        )
        .bind(command.grant_id.as_uuid())
        .bind(command.secret_id.as_uuid())
        .bind(owner_organization_id)
        .bind(target.kind)
        .bind(target.id)
        .bind(target.project_id)
        .bind(modes)
        .bind(phases)
        .bind(&command.policy.destinations)
        .bind(command.expires_at)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "grant",
            command.grant_id.as_uuid(),
            Some(command.secret_id.as_uuid()),
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "manage_grants",
            "secret.manage_grants",
            Some(command.secret_id),
            None,
            Some(command.grant_id),
            None,
            "granted",
        )
        .await?;
        append_event(
            &mut tx,
            command.grant_id.as_uuid(),
            "hephaestus.secret.granted.v1",
            "secret.granted.v1",
            json!({
                "schema_version": 1,
                "grant_id": command.grant_id,
                "secret_id": command.secret_id,
                "target_kind": target.kind,
                "target_id": target.id,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.grant_id)
    }

    /// Accepts an opaque import without resolving plaintext or ciphertext.
    ///
    /// # Errors
    ///
    /// Fails for target-side denial, inactive/expired grant, target mismatch,
    /// idempotency conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            grant_id = %command.grant_id,
            import_id = %command.import_id
        )
    )]
    pub async fn accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        command: AcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let target = resolve_target(&mut tx, command.target).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanAcceptSecretImport,
            ObjectRef::new(target.object_type, target.id),
        )
        .await?;
        if let Some((aggregate_id, _)) =
            existing_command(&mut tx, command.command_key, "accept").await?
        {
            tx.commit().await?;
            return Ok(SecretImportId::from_uuid(aggregate_id));
        }
        let grant: GrantAcceptanceRow = sqlx::query_as(
            "SELECT secret_id, owner_organization_id, target_kind, target_id
             FROM secret_grants
             WHERE id = $1 AND status = 'active'
               AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(command.grant_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        if grant.target_kind != target.kind || grant.target_id != target.id {
            return Err(SecretServiceError::TargetMismatch);
        }
        sqlx::query(
            "INSERT INTO secret_imports
             (id, grant_id, secret_id, target_kind, target_id, alias,
              status, accepted_by)
             VALUES ($1, $2, $3, $4, $5, $6, 'active', $7)",
        )
        .bind(command.import_id.as_uuid())
        .bind(command.grant_id.as_uuid())
        .bind(grant.secret_id)
        .bind(target.kind)
        .bind(target.id)
        .bind(command.alias.as_str())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "accept",
            command.import_id.as_uuid(),
            Some(command.grant_id.as_uuid()),
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            grant.owner_organization_id,
            "accept_import",
            "secret_import.accept",
            Some(SecretId::from_uuid(grant.secret_id)),
            None,
            Some(command.grant_id),
            Some(command.import_id),
            "accepted",
        )
        .await?;
        append_event(
            &mut tx,
            command.import_id.as_uuid(),
            "hephaestus.secret.imported.v1",
            "secret.imported.v1",
            json!({
                "schema_version": 1,
                "import_id": command.import_id,
                "grant_id": command.grant_id,
                "target_kind": target.kind,
                "target_id": target.id,
                "alias": command.alias,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.import_id)
    }

    /// Creates and accepts one grant/import pair in a single transaction.
    ///
    /// # Errors
    ///
    /// Fails unless the same actor independently passes source grant
    /// management and exact target import acceptance. Any denial or database
    /// failure rolls back both halves.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            secret_id = %command.secret_id,
            grant_id = %command.grant_id,
            import_id = %command.import_id
        )
    )]
    pub async fn grant_and_accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        mut command: GrantAndAcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        command.policy = command.policy.normalized()?;
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let target = resolve_target(&mut tx, command.target).await?;
        self.require(
            &mut tx,
            identity,
            Permission::ManageGrants,
            ObjectRef::new(ObjectType::Secret, command.secret_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanAcceptSecretImport,
            ObjectRef::new(target.object_type, target.id),
        )
        .await?;
        if let Some((aggregate_id, secondary_id)) =
            existing_command(&mut tx, command.command_key, "grant_accept").await?
        {
            if secondary_id != Some(command.grant_id.as_uuid()) {
                return Err(SecretServiceError::CorruptIdempotencyRecord);
            }
            tx.commit().await?;
            return Ok(SecretImportId::from_uuid(aggregate_id));
        }
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "SELECT owner_organization_id FROM secrets
             WHERE id = $1 AND status = 'active'",
        )
        .bind(command.secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        if target.organization_id != owner_organization_id {
            return Err(SecretServiceError::CrossOrganization);
        }
        let modes = command
            .policy
            .delivery_modes
            .iter()
            .map(|mode| mode_name(*mode))
            .collect::<Vec<_>>();
        let phases = command
            .policy
            .phases
            .iter()
            .map(|phase| phase_name(*phase))
            .collect::<Vec<_>>();
        sqlx::query(
            "INSERT INTO secret_grants
             (id, secret_id, owner_organization_id, target_kind, target_id,
              target_project_id, delivery_modes, phases, destinations, status,
              expires_at, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10, $11)",
        )
        .bind(command.grant_id.as_uuid())
        .bind(command.secret_id.as_uuid())
        .bind(owner_organization_id)
        .bind(target.kind)
        .bind(target.id)
        .bind(target.project_id)
        .bind(modes)
        .bind(phases)
        .bind(&command.policy.destinations)
        .bind(command.expires_at)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO secret_imports
             (id, grant_id, secret_id, target_kind, target_id, alias,
              status, accepted_by)
             VALUES ($1, $2, $3, $4, $5, $6, 'active', $7)",
        )
        .bind(command.import_id.as_uuid())
        .bind(command.grant_id.as_uuid())
        .bind(command.secret_id.as_uuid())
        .bind(target.kind)
        .bind(target.id)
        .bind(command.alias.as_str())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "grant_accept",
            command.import_id.as_uuid(),
            Some(command.grant_id.as_uuid()),
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "manage_grants",
            "secret.manage_grants",
            Some(command.secret_id),
            None,
            Some(command.grant_id),
            None,
            "granted_atomically",
        )
        .await?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "accept_import",
            "secret_import.accept",
            Some(command.secret_id),
            None,
            Some(command.grant_id),
            Some(command.import_id),
            "accepted_atomically",
        )
        .await?;
        append_event(
            &mut tx,
            command.grant_id.as_uuid(),
            "hephaestus.secret.granted.v1",
            "secret.granted.v1",
            json!({
                "schema_version": 1,
                "grant_id": command.grant_id,
                "secret_id": command.secret_id,
                "target_kind": target.kind,
                "target_id": target.id,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.import_id.as_uuid(),
            "hephaestus.secret.imported.v1",
            "secret.imported.v1",
            json!({
                "schema_version": 1,
                "import_id": command.import_id,
                "grant_id": command.grant_id,
                "target_kind": target.kind,
                "target_id": target.id,
                "alias": command.alias,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.import_id)
    }

    /// Binds an opaque import to a declared slot by creating and CAS-activating
    /// a new immutable instance revision. Existing bindings are cloned to new
    /// revision-bound identities; no historical binding is rewritten.
    ///
    /// # Errors
    ///
    /// Fails unless the actor can configure the instance and bind every
    /// carried import in its exact mode, the source grant/import/secret remain
    /// active, attachment scope matches, the release declaration accepts the
    /// request, and the active revision wins compare-and-swap.
    // The explicit validation sequence mirrors the compound authorization
    // contract and keeps revision activation atomic with every cloned binding.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            binding_id = %command.binding_id,
            instance_id = %command.instance_id,
            revision_id = %command.new_revision_id,
            import_id = %command.import_id
        )
    )]
    pub async fn bind_secret(
        &self,
        identity: &AuthenticatedIdentity,
        mut command: BindSecret,
    ) -> Result<AgentInstanceRevisionId, SecretServiceError> {
        command.phases.sort_unstable_by_key(|phase| *phase as u8);
        command.phases.dedup();
        command.attachment_ids.sort_unstable();
        command.attachment_ids.dedup();
        command.destinations.sort_unstable();
        command.destinations.dedup();
        if command.phases.is_empty() || command.destinations.len() > 32 {
            return Err(SecretServiceError::BindingPolicyMismatch);
        }
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require_binding_mode(&mut tx, identity, command.import_id, command.mode)
            .await?;
        if let Some((_binding_id, revision_id)) =
            existing_command(&mut tx, command.command_key, "bind").await?
        {
            tx.commit().await?;
            return Ok(AgentInstanceRevisionId::from_uuid(
                revision_id.ok_or(SecretServiceError::CorruptIdempotencyRecord)?,
            ));
        }
        let revision: RevisionCloneRow = sqlx::query_as(
            "SELECT instance.project_id, instance.active_revision_id,
                    revision.release_agent_id, revision.parameters,
                    revision.parameter_hash, revision.resource_selection,
                    revision.network_restriction,
                    revision.effective_runtime_policy,
                    revision.effective_policy_hash,
                    revision.platform_policy_version,
                    agent.secret_slot_schema
             FROM agent_instances AS instance
             JOIN agent_instance_revisions AS revision
               ON revision.id = instance.active_revision_id
             JOIN release_agents AS agent ON agent.id = revision.release_agent_id
             WHERE instance.id = $1
               AND instance.state IN ('active', 'update_rejected')
             FOR UPDATE OF instance",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        if revision.active_revision_id != Some(command.expected_revision_id.as_uuid()) {
            return Err(SecretServiceError::StaleInstanceRevision);
        }
        let declaration = declared_slot(&revision.secret_slot_schema, command.slot.as_str())?;
        validate_declared_binding(&declaration, &command)?;
        let import = load_eligible_import(&mut tx, command.import_id).await?;
        validate_import_policy(&import, &command)?;
        validate_binding_scope(
            &mut tx,
            command.instance_id,
            revision.project_id,
            &import,
            &command,
        )
        .await?;

        let carried: Vec<CarriedBindingRow> = sqlx::query_as(
            "SELECT import_id, slot_key, delivery_mode, phases,
                    attachment_ids, destinations, effective_policy,
                    effective_policy_hash
             FROM agent_secret_bindings
             WHERE instance_revision_id = $1 AND status = 'active'
               AND slot_key <> $2
             ORDER BY slot_key",
        )
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.slot.as_str())
        .fetch_all(&mut *tx)
        .await?;
        let mut copied = Vec::with_capacity(carried.len());
        for binding in carried {
            let mode = parse_mode(&binding.delivery_mode)?;
            let import_id = SecretImportId::from_uuid(binding.import_id);
            self.require_binding_mode(&mut tx, identity, import_id, mode)
                .await?;
            let live = load_eligible_import(&mut tx, import_id).await?;
            validate_carried_policy(&live, &binding)?;
            copied.push((AgentSecretBindingId::new(), binding));
        }
        let mut binding_ids = copied
            .iter()
            .map(|(id, _)| id.as_uuid())
            .collect::<Vec<_>>();
        binding_ids.push(command.binding_id.as_uuid());
        let diagnostics = unresolved_required_diagnostics(
            &revision.secret_slot_schema,
            copied
                .iter()
                .map(|(_, binding)| binding.slot_key.as_str())
                .chain(std::iter::once(command.slot.as_str())),
        )?;
        let runnable = diagnostics.as_array().is_some_and(std::vec::Vec::is_empty);
        sqlx::query(
            "INSERT INTO agent_instance_revisions
             (id, instance_id, release_agent_id, parameters, parameter_hash,
              secret_bindings, resource_selection, network_restriction,
              effective_runtime_policy, effective_policy_hash,
              platform_policy_version, runnable, diagnostics, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, $12, $13, $14)",
        )
        .bind(command.new_revision_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(revision.release_agent_id)
        .bind(&revision.parameters)
        .bind(&revision.parameter_hash)
        .bind(serde_json::to_value(&binding_ids)?)
        .bind(&revision.resource_selection)
        .bind(&revision.network_restriction)
        .bind(&revision.effective_runtime_policy)
        .bind(&revision.effective_policy_hash)
        .bind(&revision.platform_policy_version)
        .bind(runnable)
        .bind(&diagnostics)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        for (binding_id, binding) in copied {
            insert_binding_copy(
                &mut tx,
                binding_id,
                command.new_revision_id,
                &binding,
                identity.user_id.as_uuid(),
            )
            .await?;
        }
        let effective_policy = json!({
            "grant_id": import.grant_id,
            "secret_id": import.secret_id,
            "mode": mode_name(command.mode),
            "phases": command.phases.iter().map(|phase| phase_name(*phase)).collect::<Vec<_>>(),
            "attachment_ids": command.attachment_ids,
            "destinations": command.destinations,
        });
        let effective_hash: [u8; 32] =
            sha2::Sha256::digest(serde_json::to_vec(&effective_policy)?).into();
        sqlx::query(
            "INSERT INTO agent_secret_bindings
             (id, instance_revision_id, import_id, slot_key, delivery_mode,
              phases, attachment_ids, destinations, effective_policy,
              effective_policy_hash, status, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     'active', $11)",
        )
        .bind(command.binding_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .bind(command.import_id.as_uuid())
        .bind(command.slot.as_str())
        .bind(mode_name(command.mode))
        .bind(
            command
                .phases
                .iter()
                .map(|phase| phase_name(*phase))
                .collect::<Vec<_>>(),
        )
        .bind(&command.attachment_ids)
        .bind(&command.destinations)
        .bind(effective_policy)
        .bind(effective_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        let activated = sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $3, version = version + 1,
                 updated_at = now()
             WHERE id = $1 AND active_revision_id = $2
               AND state IN ('active', 'update_rejected')",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.expected_revision_id.as_uuid())
        .bind(command.new_revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if activated.rows_affected() != 1 {
            return Err(SecretServiceError::StaleInstanceRevision);
        }
        record_command(
            &mut tx,
            command.command_key,
            "bind",
            command.binding_id.as_uuid(),
            Some(command.new_revision_id.as_uuid()),
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            import.owner_organization_id,
            if command.mode == DeliveryMode::Raw {
                "bind_raw"
            } else {
                "bind_brokered"
            },
            if command.mode == DeliveryMode::Raw {
                "secret_import.bind_raw"
            } else {
                "secret_import.bind_brokered"
            },
            Some(SecretId::from_uuid(import.secret_id)),
            None,
            Some(SecretGrantId::from_uuid(import.grant_id)),
            Some(command.import_id),
            "revision_activated",
        )
        .await?;
        append_event(
            &mut tx,
            command.binding_id.as_uuid(),
            "hephaestus.secret.bound.v1",
            "secret.bound.v1",
            json!({
                "schema_version": 1,
                "binding_id": command.binding_id,
                "import_id": command.import_id,
                "instance_id": command.instance_id,
                "instance_revision_id": command.new_revision_id,
                "slot": command.slot,
                "delivery_mode": mode_name(command.mode),
                "runnable": runnable,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.new_revision_id)
    }

    /// Pins every live binding to an exact immutable version and issues one
    /// short-lived runtime credential immediately before guest dispatch.
    ///
    /// A retry never returns or replaces a previously minted bearer token.
    /// The caller must abandon the first run/session and create a new logical
    /// dispatch if the one-time response was lost.
    ///
    /// # Errors
    ///
    /// Fails closed when exact run, attachment, instance, release, revision,
    /// binding, import, grant, secret, or version authority is stale; when the
    /// actor cannot execute the attachment or use the release; or when a token
    /// was already issued for this command or run.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            run_id = %command.run_id,
            instance_id = %command.instance_id,
            revision_id = %command.instance_revision_id
        )
    )]
    pub async fn resolve_for_dispatch(
        &self,
        identity: &AuthenticatedIdentity,
        command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        let now = OffsetDateTime::now_utc();
        let lifetime = command.expires_at - now;
        if lifetime <= time::Duration::ZERO || lifetime > time::Duration::minutes(15) {
            return Err(SecretServiceError::InvalidLeaseLifetime);
        }
        match (
            command.phase,
            command.attachment_id,
            command.target_ref.as_ref(),
            command.target_commit.as_ref(),
        ) {
            (ExecutionPhase::Normal, Some(_), Some(_), Some(_))
            | (ExecutionPhase::Update, None, None, None) => {}
            _ => return Err(SecretServiceError::Unavailable),
        }
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        if let Some(attachment_id) = command.attachment_id {
            self.require(
                &mut tx,
                identity,
                Permission::CanExecute,
                ObjectRef::new(ObjectType::AgentAttachment, attachment_id.as_uuid()),
            )
            .await?;
        } else {
            self.require(
                &mut tx,
                identity,
                Permission::CanUpdate,
                ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
            )
            .await?;
        }
        if existing_command(&mut tx, command.command_key, "resolve")
            .await?
            .is_some()
        {
            return Err(SecretServiceError::CredentialAlreadyIssued);
        }
        let exact: DispatchRevisionRow = match command.phase {
            ExecutionPhase::Normal => {
                let attachment_id = command
                    .attachment_id
                    .ok_or(SecretServiceError::Unavailable)?;
                let target_ref = command
                    .target_ref
                    .as_ref()
                    .ok_or(SecretServiceError::Unavailable)?;
                sqlx::query_as(
                    "SELECT revision.release_agent_id, release.id AS release_id,
                            revision.parameter_hash,
                            revision.platform_policy_version,
                            attachment.repository_id
                     FROM runs AS execution
                     JOIN agent_instances AS instance
                       ON instance.id = execution.instance_id
                      AND instance.id = $2
                     JOIN agent_instance_revisions AS revision
                       ON revision.id = execution.instance_revision_id
                      AND revision.id = $3
                      AND revision.instance_id = instance.id
                     JOIN release_agents AS release_agent
                       ON release_agent.id = execution.release_agent_id
                      AND release_agent.id = revision.release_agent_id
                     JOIN releases AS release
                       ON release.id = execution.release_id
                      AND release.id = release_agent.release_id
                     JOIN agent_attachments AS attachment
                       ON attachment.id = execution.attachment_id
                      AND attachment.id = $4
                      AND attachment.instance_id = instance.id
                     WHERE execution.id = $1
                       AND execution.run_kind = 'normal'
                       AND execution.state IN (
                           'queued', 'leasing_volume', 'provisioning'
                       )
                       AND instance.state IN ('active', 'update_rejected')
                       AND instance.active_revision_id = revision.id
                       AND revision.runnable
                       AND release.state = 'published'
                       AND attachment.enabled
                       AND attachment.removed_at IS NULL
                       AND attachment.ref_selector = $5
                       AND NOT EXISTS (
                           SELECT 1 FROM run_instance_provenance
                           WHERE run_id = execution.id
                       )
                     FOR UPDATE OF execution, instance",
                )
                .bind(command.run_id.as_uuid())
                .bind(command.instance_id.as_uuid())
                .bind(command.instance_revision_id.as_uuid())
                .bind(attachment_id.as_uuid())
                .bind(target_ref.as_str())
                .fetch_optional(&mut *tx)
                .await?
            }
            ExecutionPhase::Update => {
                sqlx::query_as(
                    "SELECT revision.release_agent_id, release.id AS release_id,
                            revision.parameter_hash,
                            revision.platform_policy_version,
                            NULL::uuid AS repository_id
                     FROM runs AS execution
                     JOIN agent_updates AS update
                       ON update.hook_run_id = execution.id
                      AND update.instance_id = execution.instance_id
                      AND update.candidate_revision_id =
                          execution.instance_revision_id
                     JOIN agent_instances AS instance
                       ON instance.id = execution.instance_id
                      AND instance.id = $2
                     JOIN agent_instance_revisions AS revision
                       ON revision.id = execution.instance_revision_id
                      AND revision.id = $3
                      AND revision.instance_id = instance.id
                     JOIN release_agents AS release_agent
                       ON release_agent.id = execution.release_agent_id
                      AND release_agent.id = revision.release_agent_id
                     JOIN releases AS release
                       ON release.id = execution.release_id
                      AND release.id = release_agent.release_id
                     WHERE execution.id = $1
                       AND execution.run_kind = 'update'
                       AND execution.attachment_id IS NULL
                       AND execution.state IN (
                           'queued', 'leasing_volume', 'provisioning'
                       )
                       AND update.state = 'hook_running'
                       AND instance.state = 'updating'
                       AND NOT instance.run_gate_open
                       AND revision.runnable
                       AND release.state = 'published'
                       AND NOT EXISTS (
                           SELECT 1 FROM run_instance_provenance
                           WHERE run_id = execution.id
                       )
                     FOR UPDATE OF execution, instance",
                )
                .bind(command.run_id.as_uuid())
                .bind(command.instance_id.as_uuid())
                .bind(command.instance_revision_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?
            }
        }
        .ok_or(SecretServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, exact.release_agent_id),
        )
        .await?;

        let bindings: Vec<DispatchBindingRow> = sqlx::query_as(
            "SELECT binding.id AS binding_id, binding.slot_key,
                    binding.delivery_mode, binding.destinations,
                    binding.effective_policy_hash,
                    imported.id AS import_id, source_grant.id AS grant_id,
                    secret.id AS secret_id,
                    secret.owner_organization_id,
                    version.id AS version_id
             FROM agent_secret_bindings AS binding
             JOIN secret_imports AS imported
               ON imported.id = binding.import_id
             JOIN secret_grants AS source_grant
               ON source_grant.id = imported.grant_id
              AND source_grant.secret_id = imported.secret_id
             JOIN secrets AS secret ON secret.id = imported.secret_id
             JOIN secret_versions AS version
               ON version.id = secret.active_version_id
              AND version.secret_id = secret.id
             JOIN agent_instances AS instance ON instance.id = $3
             LEFT JOIN agent_attachments AS attachment
               ON attachment.id = $4 AND attachment.instance_id = instance.id
             WHERE binding.instance_revision_id = $1
               AND binding.status = 'active'
               AND imported.status = 'active'
               AND source_grant.status = 'active'
               AND secret.status = 'active'
               AND version.status = 'active'
               AND $2 = ANY(binding.phases)
               AND $2 = ANY(source_grant.phases)
               AND (
                   (
                       $2 = 'normal'
                       AND $4 IS NOT NULL
                       AND attachment.enabled
                       AND attachment.removed_at IS NULL
                       AND $4 = ANY(binding.attachment_ids)
                   )
                   OR (
                       $2 = 'update'
                       AND $4 IS NULL
                       AND imported.target_kind = 'project'
                       AND imported.target_id = instance.project_id
                   )
               )
               AND binding.delivery_mode = ANY(source_grant.delivery_modes)
               AND binding.delivery_mode = ANY(secret.allowed_delivery_modes)
               AND (
                   source_grant.expires_at IS NULL
                   OR source_grant.expires_at > now()
               )
               AND (
                   cardinality(source_grant.destinations) = 0
                   OR source_grant.destinations @> binding.destinations
               )
               AND (
                   (imported.target_kind = 'project'
                    AND imported.target_id = instance.project_id)
                   OR
                   (imported.target_kind = 'repository'
                    AND imported.target_id = attachment.repository_id
                    AND $2 = 'normal')
               )
               AND imported.target_kind = source_grant.target_kind
               AND imported.target_id = source_grant.target_id
             ORDER BY binding.slot_key",
        )
        .bind(command.instance_revision_id.as_uuid())
        .bind(phase_name(command.phase))
        .bind(command.instance_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .fetch_all(&mut *tx)
        .await?;
        let mut expected: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM agent_secret_bindings
             WHERE instance_revision_id = $1 AND $2 = ANY(phases)",
        )
        .bind(command.instance_revision_id.as_uuid())
        .bind(phase_name(command.phase))
        .fetch_all(&mut *tx)
        .await?;
        let mut resolved = bindings
            .iter()
            .map(|binding| binding.binding_id)
            .collect::<Vec<_>>();
        expected.sort_unstable();
        resolved.sort_unstable();
        if expected != resolved {
            return Err(SecretServiceError::Unavailable);
        }

        let mut credential_bytes = Vec::with_capacity(32);
        credential_bytes.extend_from_slice(Uuid::new_v4().as_bytes());
        credential_bytes.extend_from_slice(Uuid::new_v4().as_bytes());
        let credential = secret_domain::OpaqueRuntimeCredential::new(credential_bytes)?;
        let credential_hash = credential.storage_hash();
        sqlx::query(
            "INSERT INTO run_instance_provenance
             (run_id, instance_id, instance_revision_id, release_id,
              release_agent_id, attachment_id, target_repository_id,
              target_ref, target_commit, parameter_hash,
              platform_policy_version, phase, authorization_model_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, $12, $13)",
        )
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(exact.release_id)
        .bind(exact.release_agent_id)
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(exact.repository_id)
        .bind(command.target_ref.as_ref().map(GitRef::as_str))
        .bind(command.target_commit.as_ref().map(CommitSha::as_str))
        .bind(&exact.parameter_hash)
        .bind(&exact.platform_policy_version)
        .bind(phase_name(command.phase))
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO secret_runtime_sessions
             (id, run_id, instance_id, instance_revision_id, attachment_id,
              phase, runtime_credential_hash, status, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', $8)",
        )
        .bind(command.session_id.as_uuid())
        .bind(command.run_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(command.instance_revision_id.as_uuid())
        .bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(phase_name(command.phase))
        .bind(credential_hash.as_slice())
        .bind(command.expires_at)
        .execute(&mut *tx)
        .await?;
        let mut leases = Vec::with_capacity(bindings.len());
        for binding in bindings {
            let lease_id = SecretLeaseId::new();
            let mode = parse_mode(&binding.delivery_mode)?;
            sqlx::query(
                "INSERT INTO run_secret_provenance
                 (run_id, binding_id, secret_id, secret_version_id,
                  grant_id, import_id, authorization_model_version,
                  delivery_policy_hash)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(command.run_id.as_uuid())
            .bind(binding.binding_id)
            .bind(binding.secret_id)
            .bind(binding.version_id)
            .bind(binding.grant_id)
            .bind(binding.import_id)
            .bind(AUTHORIZATION_MODEL_VERSION)
            .bind(&binding.effective_policy_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO secret_leases
                 (id, session_id, run_id, binding_id, secret_version_id,
                  delivery_mode, slot_key, destinations, status, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'active', $9)",
            )
            .bind(lease_id.as_uuid())
            .bind(command.session_id.as_uuid())
            .bind(command.run_id.as_uuid())
            .bind(binding.binding_id)
            .bind(binding.version_id)
            .bind(&binding.delivery_mode)
            .bind(&binding.slot_key)
            .bind(&binding.destinations)
            .bind(command.expires_at)
            .execute(&mut *tx)
            .await?;
            audit_resolution(&mut tx, identity, &binding, lease_id, command.run_id).await?;
            leases.push(IssuedSecretLease {
                lease_id,
                slot: SecretSlotKey::parse(binding.slot_key)?,
                mode,
                version_id: SecretVersionId::from_uuid(binding.version_id),
            });
        }
        record_command(
            &mut tx,
            command.command_key,
            "resolve",
            command.session_id.as_uuid(),
            Some(command.run_id.as_uuid()),
            identity,
        )
        .await?;
        append_event(
            &mut tx,
            command.session_id.as_uuid(),
            "hephaestus.secret.runtime_authority_issued.v1",
            "secret.runtime_authority_issued.v1",
            json!({
                "schema_version": 1,
                "session_id": command.session_id,
                "run_id": command.run_id,
                "instance_id": command.instance_id,
                "instance_revision_id": command.instance_revision_id,
                "attachment_id": command.attachment_id,
                "phase": phase_name(command.phase),
                "lease_count": leases.len(),
                "expires_at": command.expires_at,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(RuntimeSecretAuthority {
            session_id: command.session_id,
            credential,
            leases,
        })
    }

    /// Enables or disables later secret resolution without changing versions.
    ///
    /// Disabling is immediate for new leases and requires revocation
    /// authority. Re-enabling requires rotation authority and does not restore
    /// any separately revoked grant, import, binding, session, or lease.
    ///
    /// # Errors
    ///
    /// Fails for denial, invalid lifecycle, idempotency conflict, or database
    /// failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            %secret_id,
            enabled
        )
    )]
    pub async fn set_secret_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: SecretCommandKey,
        secret_id: SecretId,
        enabled: bool,
    ) -> Result<(), SecretServiceError> {
        let operation = if enabled { "enable" } else { "disable" };
        let permission = if enabled {
            Permission::Rotate
        } else {
            Permission::Revoke
        };
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            permission,
            ObjectRef::new(ObjectType::Secret, secret_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, operation)
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let expected = if enabled { "disabled" } else { "active" };
        let next = if enabled { "active" } else { "disabled" };
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "UPDATE secrets SET status = $2, updated_at = now()
             WHERE id = $1 AND status = $3
             RETURNING owner_organization_id",
        )
        .bind(secret_id.as_uuid())
        .bind(next)
        .bind(expected)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command_key,
            operation,
            secret_id.as_uuid(),
            None,
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            operation,
            permission.as_str(),
            Some(secret_id),
            None,
            None,
            None,
            if enabled {
                "later_resolution_enabled"
            } else {
                "later_resolution_disabled"
            },
        )
        .await?;
        append_event(
            &mut tx,
            secret_id.as_uuid(),
            if enabled {
                "hephaestus.secret.enabled.v1"
            } else {
                "hephaestus.secret.disabled.v1"
            },
            if enabled {
                "secret.enabled.v1"
            } else {
                "secret.disabled.v1"
            },
            json!({
                "schema_version": 1,
                "secret_id": secret_id,
                "status": next,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Revokes a secret and all downstream active authority immediately.
    ///
    /// Active raw leases are marked honestly as potentially observed and their
    /// runs are included in the reconciliation outbox event.
    ///
    /// # Errors
    ///
    /// Fails for denial, missing secret, idempotency conflict, or database
    /// failure.
    // The downstream revocations intentionally remain visible as one atomic
    // reconciliation transaction.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            %secret_id
        )
    )]
    pub async fn revoke_secret(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: SecretCommandKey,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::Revoke,
            ObjectRef::new(ObjectType::Secret, secret_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "revoke")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "UPDATE secrets
             SET status = 'revoked', revoked_at = now(), updated_at = now()
             WHERE id = $1 AND status IN ('active', 'disabled')
             RETURNING owner_organization_id",
        )
        .bind(secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        sqlx::query(
            "UPDATE secret_grants SET status = 'revoked', revoked_at = now()
             WHERE secret_id = $1 AND status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE secret_imports SET status = 'revoked', revoked_at = now()
             WHERE secret_id = $1 AND status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE agent_secret_bindings AS binding
             SET status = 'revoked', revoked_at = now()
             FROM secret_imports AS imported
             WHERE binding.import_id = imported.id
               AND imported.secret_id = $1
               AND binding.status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE secret_leases AS lease
             SET status = 'revoked', revoked_at = now(),
                 raw_material_observed =
                     raw_material_observed OR lease.delivery_mode = 'raw'
             FROM secret_versions AS version
             WHERE lease.secret_version_id = version.id
               AND version.secret_id = $1
               AND lease.status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE secret_runtime_sessions AS session
             SET status = 'revoked', revoked_at = now()
             WHERE session.status = 'active'
               AND EXISTS (
                   SELECT 1
                   FROM secret_leases AS lease
                   JOIN secret_versions AS version
                     ON version.id = lease.secret_version_id
                   WHERE lease.session_id = session.id
                     AND version.secret_id = $1
               )",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command_key,
            "revoke",
            secret_id.as_uuid(),
            None,
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "revoke",
            "secret.revoke",
            Some(secret_id),
            None,
            None,
            None,
            "revoked_and_reconciliation_requested",
        )
        .await?;
        append_event(
            &mut tx,
            secret_id.as_uuid(),
            "hephaestus.secret.reconcile_revocation.v1",
            "secret.reconcile_revocation.v1",
            json!({
                "schema_version": 1,
                "secret_id": secret_id,
                "cancel_affected_raw_guests": true,
                "stop_broker_leases": true,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Tombstones a revoked secret and purges all encrypted version material
    /// only when no active lease retains it.
    ///
    /// # Errors
    ///
    /// Fails for denial, active leases, invalid lifecycle, idempotency
    /// conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            %secret_id
        )
    )]
    pub async fn purge_secret(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: SecretCommandKey,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::Purge,
            ObjectRef::new(ObjectType::Secret, secret_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "purge")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let row: (Uuid, String) = sqlx::query_as(
            "SELECT owner_organization_id, status FROM secrets
             WHERE id = $1 FOR UPDATE",
        )
        .bind(secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        if !matches!(row.1.as_str(), "revoked" | "tombstoned") {
            return Err(SecretServiceError::InvalidLifecycle);
        }
        let active_leases: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM secret_leases AS lease
             JOIN secret_versions AS version
               ON version.id = lease.secret_version_id
             WHERE version.secret_id = $1
               AND lease.status = 'active' AND lease.expires_at > now()",
        )
        .bind(secret_id.as_uuid())
        .fetch_one(&mut *tx)
        .await?;
        if active_leases != 0 {
            return Err(SecretServiceError::ActiveLeases);
        }
        sqlx::query(
            "UPDATE secrets SET status = 'tombstoned',
                    tombstoned_at = COALESCE(tombstoned_at, now()),
                    active_version_id = NULL, updated_at = now()
             WHERE id = $1",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE secret_versions
             SET status = 'purged', data_nonce = NULL, ciphertext = NULL,
                 wrap_nonce = NULL, wrapped_data_key = NULL,
                 associated_data_hash = NULL, purged_at = now()
             WHERE secret_id = $1 AND status <> 'purged'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE secrets SET status = 'purged', purged_at = now(),
                    updated_at = now() WHERE id = $1",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command_key,
            "purge",
            secret_id.as_uuid(),
            None,
            identity,
        )
        .await?;
        audit(
            &mut tx,
            identity,
            row.0,
            "purge",
            "secret.purge",
            Some(secret_id),
            None,
            None,
            None,
            "cryptographic_material_purged",
        )
        .await?;
        append_event(
            &mut tx,
            secret_id.as_uuid(),
            "hephaestus.secret.purged.v1",
            "secret.purged.v1",
            json!({"schema_version": 1, "secret_id": secret_id}),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), SecretServiceError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // Keep rejected attempts durable while allowing the caller's
            // command transaction to roll back all domain changes.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity).await?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                permission,
                object,
                decision,
                identity.request_id,
            )
            .await?;
            audit_tx.commit().await?;
            Err(SecretServiceError::AuthorizationDenied)
        }
    }

    async fn require_binding_mode(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        import_id: SecretImportId,
        mode: DeliveryMode,
    ) -> Result<(), SecretServiceError> {
        self.require(
            tx,
            identity,
            match mode {
                DeliveryMode::Raw => Permission::BindRaw,
                DeliveryMode::Brokered => Permission::BindBrokered,
            },
            ObjectRef::new(ObjectType::SecretImport, import_id.as_uuid()),
        )
        .await
    }
}

impl<K: KeyProvider + Send + Sync> SecretRuntimeService<K> {
    /// Creates an agent-facing service with separate authorization and
    /// ciphertext-resolver connections.
    #[must_use]
    pub fn new(
        authorization_pool: PgPool,
        resolver_pool: PgPool,
        encrypted_store: EncryptedStore<K>,
        authorizer: Arc<dyn Authorizer>,
    ) -> Self {
        Self {
            authorization_pool,
            resolver_pool,
            encrypted_store,
            authorizer,
        }
    }

    /// Authenticates and resolves one exact raw lease for ephemeral mounting.
    ///
    /// # Errors
    ///
    /// Fails closed for token/run/slot mismatch, expiry, revocation,
    /// authorization denial, lifecycle changes, tampering, or unavailable
    /// encryption keys.
    #[tracing::instrument(skip_all, fields(run_id = %claimed_run_id, slot = slot.as_str()))]
    pub async fn receive_raw(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        claimed_run_id: RunId,
        slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        let session = self
            .authenticate_session(credential, claimed_run_id)
            .await?;
        let lease = self
            .authorize_runtime_lease(&session, &slot, DeliveryMode::Raw, Permission::ReceiveRaw)
            .await?;
        let (context, encrypted) = load_runtime_version(
            &self.resolver_pool,
            &session,
            &lease,
            DeliveryMode::Raw,
            true,
            "raw_materialized",
        )
        .await?;
        let value = self.encrypted_store.resolve(&context, &encrypted)?;
        Ok(ResolvedRawSecret {
            lease_id: SecretLeaseId::from_uuid(lease.lease_id),
            slot,
            value,
        })
    }

    /// Executes one semantic broker operation without returning plaintext to
    /// the guest.
    ///
    /// # Errors
    ///
    /// Fails closed for invalid bounded input, token theft, destination or
    /// capability mismatch, live authorization/lifecycle denial, encrypted
    /// value failure, adapter failure, or response overflow.
    #[tracing::instrument(
        skip_all,
        fields(
            run_id = %request.run_id,
            slot = request.slot.as_str(),
            destination = request.destination.as_str(),
            operation = request.operation.as_str()
        )
    )]
    pub async fn use_brokered<A: BrokerAdapter + ?Sized>(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        adapter: &A,
    ) -> Result<BrokerResponse, SecretServiceError> {
        validate_broker_request(request)?;
        let session = self
            .authenticate_session(credential, request.run_id)
            .await?;
        let lease = self
            .authorize_runtime_lease(
                &session,
                &request.slot,
                DeliveryMode::Brokered,
                Permission::UseBrokered,
            )
            .await?;
        if !lease.destinations.is_empty()
            && !lease
                .destinations
                .iter()
                .any(|value| value == &request.destination)
        {
            return Err(SecretServiceError::BrokerRequestDenied);
        }
        let (context, encrypted) = load_runtime_version(
            &self.resolver_pool,
            &session,
            &lease,
            DeliveryMode::Brokered,
            false,
            "broker_call_started",
        )
        .await?;
        let value = self.encrypted_store.resolve(&context, &encrypted)?;
        let response = adapter
            .invoke(
                &value,
                &request.destination,
                &request.operation,
                &request.body,
            )
            .await
            .map_err(SecretServiceError::BrokerAdapter)?;
        if response.body.len() > 65_536 {
            return Err(SecretServiceError::BrokerResponseTooLarge);
        }
        // A fresh live check prevents a response from being delivered after a
        // revocation that completed while the upstream operation was active.
        self.authorize_runtime_lease(
            &session,
            &request.slot,
            DeliveryMode::Brokered,
            Permission::UseBrokered,
        )
        .await?;
        record_runtime_use(
            &self.resolver_pool,
            &session,
            &lease,
            "broker_call_completed",
        )
        .await?;
        Ok(response)
    }

    async fn authenticate_session(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        claimed_run_id: RunId,
    ) -> Result<RuntimeSessionRow, SecretServiceError> {
        let hash = credential.storage_hash();
        let session: RuntimeSessionRow = sqlx::query_as(
            "SELECT session_id, run_id, instance_id, instance_revision_id,
                    attachment_id, phase, expires_at
             FROM authenticate_secret_runtime($1)",
        )
        .bind(hash.as_slice())
        .fetch_optional(&self.authorization_pool)
        .await?
        .ok_or(SecretServiceError::RuntimeAuthenticationDenied)?;
        if session.run_id != claimed_run_id.as_uuid()
            || session.expires_at <= OffsetDateTime::now_utc()
        {
            return Err(SecretServiceError::RuntimeAuthenticationDenied);
        }
        Ok(session)
    }

    async fn authorize_runtime_lease(
        &self,
        session: &RuntimeSessionRow,
        slot: &SecretSlotKey,
        mode: DeliveryMode,
        permission: Permission,
    ) -> Result<RuntimeLeaseAuthorizationRow, SecretServiceError> {
        let run_id = RunId::from_uuid(session.run_id);
        let mut tx = begin_runtime_transaction(&self.authorization_pool, run_id).await?;
        let lease: RuntimeLeaseAuthorizationRow = sqlx::query_as(
            "SELECT lease.id AS lease_id, lease.secret_version_id,
                    lease.destinations
             FROM secret_leases AS lease
             WHERE lease.session_id = $1 AND lease.run_id = $2
               AND lease.slot_key = $3 AND lease.delivery_mode = $4
               AND lease.status = 'active' AND lease.expires_at > now()",
        )
        .bind(session.session_id)
        .bind(session.run_id)
        .bind(slot.as_str())
        .bind(mode_name(mode))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SecretServiceError::Unavailable)?;
        let decision = self
            .authorizer
            .check(
                &mut tx,
                Subject::Run(run_id),
                permission,
                ObjectRef::new(ObjectType::SecretLease, lease.lease_id),
            )
            .await?;
        if decision != AuthorizationDecision::Allow {
            return Err(SecretServiceError::AuthorizationDenied);
        }
        tx.commit().await?;
        Ok(lease)
    }
}

#[derive(sqlx::FromRow)]
struct RevisionCloneRow {
    project_id: Uuid,
    active_revision_id: Option<Uuid>,
    release_agent_id: Uuid,
    parameters: serde_json::Value,
    parameter_hash: Vec<u8>,
    resource_selection: serde_json::Value,
    network_restriction: serde_json::Value,
    effective_runtime_policy: serde_json::Value,
    effective_policy_hash: Vec<u8>,
    platform_policy_version: String,
    secret_slot_schema: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct EligibleImportRow {
    grant_id: Uuid,
    secret_id: Uuid,
    owner_organization_id: Uuid,
    target_kind: String,
    target_id: Uuid,
    delivery_modes: Vec<String>,
    phases: Vec<String>,
    destinations: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct CarriedBindingRow {
    import_id: Uuid,
    slot_key: String,
    delivery_mode: String,
    phases: Vec<String>,
    attachment_ids: Vec<Uuid>,
    destinations: Vec<String>,
    effective_policy: serde_json::Value,
    effective_policy_hash: Vec<u8>,
}

#[derive(sqlx::FromRow)]
struct DispatchRevisionRow {
    release_agent_id: Uuid,
    release_id: Uuid,
    parameter_hash: Vec<u8>,
    platform_policy_version: String,
    repository_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct DispatchBindingRow {
    binding_id: Uuid,
    slot_key: String,
    delivery_mode: String,
    destinations: Vec<String>,
    effective_policy_hash: Vec<u8>,
    import_id: Uuid,
    grant_id: Uuid,
    secret_id: Uuid,
    owner_organization_id: Uuid,
    version_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct RuntimeSessionRow {
    session_id: Uuid,
    run_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    attachment_id: Option<Uuid>,
    phase: String,
    expires_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct RuntimeLeaseAuthorizationRow {
    lease_id: Uuid,
    secret_version_id: Uuid,
    destinations: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct RuntimeEncryptedVersionRow {
    secret_id: Uuid,
    version_id: Uuid,
    sequence: i64,
    organization_id: Option<Uuid>,
    project_id: Option<Uuid>,
    algorithm: String,
    key_reference: String,
    data_nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    wrap_nonce: Vec<u8>,
    wrapped_data_key: Vec<u8>,
    associated_data_hash: Vec<u8>,
    content_length: i32,
}

struct DeclaredSlot {
    delivery_modes: Vec<String>,
    phases: Vec<String>,
    destinations: Vec<String>,
}

fn declared_slot(
    schema: &serde_json::Value,
    slot_key: &str,
) -> Result<DeclaredSlot, SecretServiceError> {
    let declaration = schema
        .as_array()
        .and_then(|slots| {
            slots
                .iter()
                .find(|slot| slot.get("key").and_then(serde_json::Value::as_str) == Some(slot_key))
        })
        .ok_or(SecretServiceError::SlotNotDeclared)?;
    Ok(DeclaredSlot {
        delivery_modes: string_array(declaration, "delivery_modes")?,
        phases: string_array(declaration, "phases")?,
        destinations: string_array(declaration, "destinations")?,
    })
}

fn string_array(value: &serde_json::Value, field: &str) -> Result<Vec<String>, SecretServiceError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or(SecretServiceError::InvalidStoredData)?
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .ok_or(SecretServiceError::InvalidStoredData)
        })
        .collect()
}

fn validate_declared_binding(
    declaration: &DeclaredSlot,
    command: &BindSecret,
) -> Result<(), SecretServiceError> {
    if !declaration
        .delivery_modes
        .iter()
        .any(|mode| mode == mode_name(command.mode))
        || command.phases.iter().any(|phase| {
            !declaration
                .phases
                .iter()
                .any(|value| value == phase_name(*phase))
        })
        || (!declaration.destinations.is_empty()
            && command
                .destinations
                .iter()
                .any(|destination| !declaration.destinations.contains(destination)))
    {
        return Err(SecretServiceError::BindingPolicyMismatch);
    }
    Ok(())
}

async fn load_eligible_import(
    tx: &mut Transaction<'_, Postgres>,
    import_id: SecretImportId,
) -> Result<EligibleImportRow, SecretServiceError> {
    sqlx::query_as(
        "SELECT source_grant.id AS grant_id, secret.id AS secret_id,
                secret.owner_organization_id, imported.target_kind,
                imported.target_id, source_grant.delivery_modes,
                source_grant.phases, source_grant.destinations
         FROM secret_imports AS imported
         JOIN secret_grants AS source_grant
           ON source_grant.id = imported.grant_id
         JOIN secrets AS secret ON secret.id = imported.secret_id
         WHERE imported.id = $1 AND imported.status = 'active'
           AND source_grant.status = 'active' AND secret.status = 'active'
           AND (
               source_grant.expires_at IS NULL
               OR source_grant.expires_at > now()
           )",
    )
    .bind(import_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(SecretServiceError::Unavailable)
}

fn validate_import_policy(
    import: &EligibleImportRow,
    command: &BindSecret,
) -> Result<(), SecretServiceError> {
    if !import
        .delivery_modes
        .iter()
        .any(|mode| mode == mode_name(command.mode))
        || command.phases.iter().any(|phase| {
            !import
                .phases
                .iter()
                .any(|value| value == phase_name(*phase))
        })
        || (!import.destinations.is_empty()
            && command
                .destinations
                .iter()
                .any(|destination| !import.destinations.contains(destination)))
    {
        return Err(SecretServiceError::BindingPolicyMismatch);
    }
    Ok(())
}

async fn validate_binding_scope(
    tx: &mut Transaction<'_, Postgres>,
    instance_id: AgentInstanceId,
    project_id: Uuid,
    import: &EligibleImportRow,
    command: &BindSecret,
) -> Result<(), SecretServiceError> {
    let includes_normal = command.phases.contains(&ExecutionPhase::Normal);
    let includes_update = command.phases.contains(&ExecutionPhase::Update);
    if includes_normal && command.attachment_ids.is_empty() {
        return Err(SecretServiceError::BindingOutOfScope);
    }
    if import.target_kind == "project" {
        if import.target_id != project_id {
            return Err(SecretServiceError::BindingOutOfScope);
        }
    } else if import.target_kind == "repository" {
        if command.attachment_ids.is_empty() || includes_update {
            return Err(SecretServiceError::BindingOutOfScope);
        }
    } else {
        return Err(SecretServiceError::InvalidStoredData);
    }
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, repository_id FROM agent_attachments
         WHERE instance_id = $1 AND id = ANY($2)
           AND enabled AND removed_at IS NULL",
    )
    .bind(instance_id.as_uuid())
    .bind(&command.attachment_ids)
    .fetch_all(&mut **tx)
    .await?;
    if rows.len() != command.attachment_ids.len()
        || (import.target_kind == "repository"
            && rows
                .iter()
                .any(|(_, repository_id)| *repository_id != import.target_id))
    {
        return Err(SecretServiceError::BindingOutOfScope);
    }
    Ok(())
}

fn validate_carried_policy(
    import: &EligibleImportRow,
    binding: &CarriedBindingRow,
) -> Result<(), SecretServiceError> {
    if !import.delivery_modes.contains(&binding.delivery_mode)
        || binding
            .phases
            .iter()
            .any(|phase| !import.phases.contains(phase))
        || (!import.destinations.is_empty()
            && binding
                .destinations
                .iter()
                .any(|destination| !import.destinations.contains(destination)))
    {
        return Err(SecretServiceError::BindingPolicyMismatch);
    }
    Ok(())
}

fn unresolved_required_diagnostics<'a>(
    schema: &serde_json::Value,
    bound_slots: impl Iterator<Item = &'a str>,
) -> Result<serde_json::Value, SecretServiceError> {
    let bound = bound_slots.collect::<std::collections::HashSet<_>>();
    let diagnostics = schema
        .as_array()
        .ok_or(SecretServiceError::InvalidStoredData)?
        .iter()
        .filter(|slot| slot.get("required").and_then(serde_json::Value::as_bool) == Some(true))
        .filter_map(|slot| slot.get("key").and_then(serde_json::Value::as_str))
        .filter(|slot| !bound.contains(slot))
        .map(|slot| {
            json!({
                "code": "required_secret_binding_missing",
                "field": format!("secret_slots.{slot}")
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::Value::Array(diagnostics))
}

async fn insert_binding_copy(
    tx: &mut Transaction<'_, Postgres>,
    binding_id: AgentSecretBindingId,
    revision_id: AgentInstanceRevisionId,
    binding: &CarriedBindingRow,
    creator_id: Uuid,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO agent_secret_bindings
         (id, instance_revision_id, import_id, slot_key, delivery_mode,
          phases, attachment_ids, destinations, effective_policy,
          effective_policy_hash, status, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 'active', $11)",
    )
    .bind(binding_id.as_uuid())
    .bind(revision_id.as_uuid())
    .bind(binding.import_id)
    .bind(&binding.slot_key)
    .bind(&binding.delivery_mode)
    .bind(&binding.phases)
    .bind(&binding.attachment_ids)
    .bind(&binding.destinations)
    .bind(&binding.effective_policy)
    .bind(&binding.effective_policy_hash)
    .bind(creator_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn load_runtime_version(
    resolver_pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    mode: DeliveryMode,
    raw_observed: bool,
    outcome: &str,
) -> Result<(VersionContext, EncryptedSecretVersion), SecretServiceError> {
    let mut tx = resolver_pool.begin().await?;
    let row: RuntimeEncryptedVersionRow = sqlx::query_as(
        "SELECT secret.id AS secret_id, version.id AS version_id,
                version.sequence, secret.organization_id, secret.project_id,
                version.algorithm, version.key_reference, version.data_nonce,
                version.ciphertext, version.wrap_nonce,
                version.wrapped_data_key, version.associated_data_hash,
                version.content_length
         FROM secret_leases AS lease
         JOIN secret_runtime_sessions AS session
           ON session.id = lease.session_id AND session.run_id = lease.run_id
         JOIN run_instance_provenance AS run_provenance
           ON run_provenance.run_id = lease.run_id
         JOIN run_secret_provenance AS secret_provenance
           ON secret_provenance.run_id = lease.run_id
          AND secret_provenance.binding_id = lease.binding_id
          AND secret_provenance.secret_version_id = lease.secret_version_id
         JOIN agent_secret_bindings AS binding
           ON binding.id = lease.binding_id
         JOIN secret_imports AS imported
           ON imported.id = secret_provenance.import_id
          AND imported.id = binding.import_id
         JOIN secret_grants AS source_grant
           ON source_grant.id = secret_provenance.grant_id
          AND source_grant.id = imported.grant_id
         JOIN secrets AS secret
           ON secret.id = secret_provenance.secret_id
          AND secret.id = imported.secret_id
         JOIN secret_versions AS version
           ON version.id = lease.secret_version_id
          AND version.secret_id = secret.id
         WHERE lease.id = $1 AND lease.secret_version_id = $2
           AND lease.session_id = $3 AND lease.run_id = $4
           AND lease.delivery_mode = $5
           AND lease.status = 'active' AND lease.expires_at > now()
           AND session.status = 'active' AND session.expires_at > now()
           AND session.instance_id = $6
           AND session.instance_revision_id = $7
           AND session.attachment_id IS NOT DISTINCT FROM $8
           AND session.phase = $9
           AND run_provenance.instance_id = session.instance_id
           AND run_provenance.instance_revision_id = session.instance_revision_id
           AND run_provenance.attachment_id
               IS NOT DISTINCT FROM session.attachment_id
           AND run_provenance.phase = session.phase
           AND binding.status = 'active'
           AND imported.status = 'active'
           AND source_grant.status = 'active'
           AND secret.status = 'active'
           AND version.status = 'active'
           AND (source_grant.expires_at IS NULL
                OR source_grant.expires_at > now())",
    )
    .bind(lease.lease_id)
    .bind(lease.secret_version_id)
    .bind(session.session_id)
    .bind(session.run_id)
    .bind(mode_name(mode))
    .bind(session.instance_id)
    .bind(session.instance_revision_id)
    .bind(session.attachment_id)
    .bind(&session.phase)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(SecretServiceError::Unavailable)?;
    if raw_observed {
        sqlx::query(
            "UPDATE secret_leases
             SET raw_material_observed = true
             WHERE id = $1 AND status = 'active'",
        )
        .bind(lease.lease_id)
        .execute(&mut *tx)
        .await?;
    }
    record_runtime_use_tx(&mut tx, session, lease, mode, outcome).await?;
    tx.commit().await?;
    encrypted_version(row)
}

fn encrypted_version(
    row: RuntimeEncryptedVersionRow,
) -> Result<(VersionContext, EncryptedSecretVersion), SecretServiceError> {
    let owner = match (row.organization_id, row.project_id) {
        (Some(id), None) => SecretOwner::Organization(OrganizationId::from_uuid(id)),
        (None, Some(id)) => SecretOwner::Project(ProjectId::from_uuid(id)),
        _ => return Err(SecretServiceError::InvalidStoredData),
    };
    let version_id = SecretVersionId::from_uuid(row.version_id);
    let context = VersionContext {
        owner,
        secret_id: SecretId::from_uuid(row.secret_id),
        version_id,
        sequence: u64::try_from(row.sequence).map_err(|_| SecretServiceError::InvalidStoredData)?,
        media_type: String::from("application/octet-stream"),
    };
    let encrypted = EncryptedSecretVersion {
        version_id,
        algorithm: row.algorithm,
        key_reference: row.key_reference,
        data_nonce: row
            .data_nonce
            .try_into()
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        ciphertext: row.ciphertext,
        wrap_nonce: row
            .wrap_nonce
            .try_into()
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        wrapped_data_key: row.wrapped_data_key,
        associated_data_hash: row
            .associated_data_hash
            .try_into()
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        content_length: u32::try_from(row.content_length)
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
    };
    Ok((context, encrypted))
}

async fn record_runtime_use(
    resolver_pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    let mut tx = resolver_pool.begin().await?;
    record_runtime_use_tx(&mut tx, session, lease, DeliveryMode::Brokered, outcome).await?;
    tx.commit().await?;
    Ok(())
}

async fn record_runtime_use_tx(
    tx: &mut Transaction<'_, Postgres>,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    mode: DeliveryMode,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    let inserted = sqlx::query(
        "INSERT INTO secret_audit_events
         (id, owner_organization_id, runtime_run_id, secret_id,
          secret_version_id, grant_id, import_id, binding_id, lease_id,
          operation, permission, delivery_mode, decision, outcome,
          authorization_model_version, policy_version)
         SELECT $1, secret.owner_organization_id, $2, provenance.secret_id,
                provenance.secret_version_id, provenance.grant_id,
                provenance.import_id, provenance.binding_id, lease.id,
                $3, $4, $5, 'allow', $6, $7, 'runtime/v1'
         FROM secret_leases AS lease
         JOIN run_secret_provenance AS provenance
           ON provenance.run_id = lease.run_id
          AND provenance.binding_id = lease.binding_id
         JOIN secrets AS secret ON secret.id = provenance.secret_id
         WHERE lease.id = $8 AND lease.session_id = $9
           AND lease.run_id = $2",
    )
    .bind(Uuid::new_v4())
    .bind(session.run_id)
    .bind(if mode == DeliveryMode::Raw {
        "receive_raw"
    } else {
        "use_brokered"
    })
    .bind(if mode == DeliveryMode::Raw {
        "secret.receive_raw"
    } else {
        "secret.use_brokered"
    })
    .bind(mode_name(mode))
    .bind(outcome)
    .bind(AUTHORIZATION_MODEL_VERSION)
    .bind(lease.lease_id)
    .bind(session.session_id)
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() != 1 {
        return Err(SecretServiceError::Unavailable);
    }
    Ok(())
}

fn validate_broker_request(request: &BrokerRequest) -> Result<(), SecretServiceError> {
    let valid_operation = (1..=64).contains(&request.operation.len())
        && request
            .operation
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit());
    let valid_destination = (1..=253).contains(&request.destination.len())
        && request.destination.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
        })
        && request.destination.parse::<std::net::IpAddr>().is_err()
        && request.destination.rsplit('.').next() != Some("local")
        && request.destination != "localhost";
    if !valid_operation || !valid_destination || request.body.len() > 65_536 {
        return Err(SecretServiceError::BrokerRequestDenied);
    }
    Ok(())
}

async fn audit_resolution(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    binding: &DispatchBindingRow,
    lease_id: SecretLeaseId,
    run_id: RunId,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO secret_audit_events
         (id, owner_organization_id, requester_id, runtime_run_id,
          secret_id, secret_version_id, grant_id, import_id, binding_id,
          lease_id, operation, permission, delivery_mode, decision, outcome,
          request_id, authorization_model_version, policy_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 'resolve', $11, $12, 'allow', 'lease_issued',
                 $13, $14, 'dispatch/v1')",
    )
    .bind(Uuid::new_v4())
    .bind(binding.owner_organization_id)
    .bind(identity.user_id.as_uuid())
    .bind(run_id.as_uuid())
    .bind(binding.secret_id)
    .bind(binding.version_id)
    .bind(binding.grant_id)
    .bind(binding.import_id)
    .bind(binding.binding_id)
    .bind(lease_id.as_uuid())
    .bind(if binding.delivery_mode == "raw" {
        "secret.receive_raw"
    } else {
        "secret.use_brokered"
    })
    .bind(&binding.delivery_mode)
    .bind(identity.request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn parse_mode(value: &str) -> Result<DeliveryMode, SecretServiceError> {
    match value {
        "raw" => Ok(DeliveryMode::Raw),
        "brokered" => Ok(DeliveryMode::Brokered),
        _ => Err(SecretServiceError::InvalidStoredData),
    }
}

#[derive(sqlx::FromRow)]
struct SecretRotationRow {
    owner_organization_id: Uuid,
    organization_id: Option<Uuid>,
    project_id: Option<Uuid>,
    active_version_id: Option<Uuid>,
    sequence: i64,
}

impl SecretRotationRow {
    const fn owner(&self) -> Result<SecretOwner, SecretServiceError> {
        match (self.organization_id, self.project_id) {
            (Some(id), None) => Ok(SecretOwner::Organization(OrganizationId::from_uuid(id))),
            (None, Some(id)) => Ok(SecretOwner::Project(ProjectId::from_uuid(id))),
            _ => Err(SecretServiceError::InvalidStoredData),
        }
    }
}

#[derive(sqlx::FromRow)]
struct GrantAcceptanceRow {
    secret_id: Uuid,
    owner_organization_id: Uuid,
    target_kind: String,
    target_id: Uuid,
}

struct ResolvedTarget {
    kind: &'static str,
    object_type: ObjectType,
    id: Uuid,
    project_id: Uuid,
    organization_id: Uuid,
}

async fn resolve_target(
    tx: &mut Transaction<'_, Postgres>,
    target: SecretTarget,
) -> Result<ResolvedTarget, SecretServiceError> {
    match target {
        SecretTarget::Project(id) => {
            let organization_id: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
                    .bind(id.as_uuid())
                    .fetch_optional(&mut **tx)
                    .await?
                    .ok_or(SecretServiceError::Unavailable)?;
            Ok(ResolvedTarget {
                kind: "project",
                object_type: ObjectType::Project,
                id: id.as_uuid(),
                project_id: id.as_uuid(),
                organization_id,
            })
        }
        SecretTarget::Repository(id) => {
            let row: (Uuid, Uuid) = sqlx::query_as(
                "SELECT repositories.project_id, projects.organization_id
                 FROM repositories
                 JOIN projects ON projects.id = repositories.project_id
                 WHERE repositories.id = $1",
            )
            .bind(id.as_uuid())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(SecretServiceError::Unavailable)?;
            Ok(ResolvedTarget {
                kind: "repository",
                object_type: ObjectType::Repository,
                id: id.as_uuid(),
                project_id: row.0,
                organization_id: row.1,
            })
        }
    }
}

async fn resolve_owner(
    tx: &mut Transaction<'_, Postgres>,
    owner: SecretOwner,
) -> Result<(ObjectType, Uuid, Uuid, Option<Uuid>, Option<Uuid>), SecretServiceError> {
    match owner {
        SecretOwner::Organization(id) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM organizations WHERE id = $1)")
                    .bind(id.as_uuid())
                    .fetch_one(&mut **tx)
                    .await?;
            if !exists {
                return Err(SecretServiceError::Unavailable);
            }
            Ok((
                ObjectType::Organization,
                id.as_uuid(),
                id.as_uuid(),
                None,
                Some(id.as_uuid()),
            ))
        }
        SecretOwner::Project(id) => {
            let organization_id: Uuid =
                sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
                    .bind(id.as_uuid())
                    .fetch_optional(&mut **tx)
                    .await?
                    .ok_or(SecretServiceError::Unavailable)?;
            Ok((
                ObjectType::Project,
                id.as_uuid(),
                organization_id,
                Some(id.as_uuid()),
                None,
            ))
        }
    }
}

fn normalized_modes(modes: &[DeliveryMode]) -> Result<Vec<&'static str>, SecretServiceError> {
    let mut names = modes.iter().copied().map(mode_name).collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        return Err(SecretServiceError::InvalidDeliveryModes);
    }
    Ok(names)
}

const fn mode_name(mode: DeliveryMode) -> &'static str {
    match mode {
        DeliveryMode::Raw => "raw",
        DeliveryMode::Brokered => "brokered",
    }
}

const fn phase_name(phase: ExecutionPhase) -> &'static str {
    match phase {
        ExecutionPhase::Normal => "normal",
        ExecutionPhase::Update => "update",
    }
}

async fn insert_encrypted_version(
    tx: &mut Transaction<'_, Postgres>,
    secret_id: SecretId,
    sequence: u64,
    encrypted: &EncryptedSecretVersion,
    creator_id: Uuid,
) -> Result<(), SecretServiceError> {
    let sequence =
        i64::try_from(sequence).map_err(|_| SecretServiceError::VersionSequenceExhausted)?;
    sqlx::query(
        "INSERT INTO secret_versions
         (id, secret_id, sequence, status, algorithm, key_reference,
          data_nonce, ciphertext, wrap_nonce, wrapped_data_key,
          associated_data_hash, content_length, created_by)
         VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(encrypted.version_id.as_uuid())
    .bind(secret_id.as_uuid())
    .bind(sequence)
    .bind(&encrypted.algorithm)
    .bind(&encrypted.key_reference)
    .bind(encrypted.data_nonce.as_slice())
    .bind(&encrypted.ciphertext)
    .bind(encrypted.wrap_nonce.as_slice())
    .bind(&encrypted.wrapped_data_key)
    .bind(encrypted.associated_data_hash.as_slice())
    .bind(i32::try_from(encrypted.content_length).unwrap_or(i32::MAX))
    .bind(creator_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn existing_command(
    tx: &mut Transaction<'_, Postgres>,
    command_key: SecretCommandKey,
    operation: &str,
) -> Result<Option<(Uuid, Option<Uuid>)>, SecretServiceError> {
    let row: Option<(String, Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT operation, aggregate_id, secondary_id
         FROM secret_command_inbox WHERE command_key = $1",
    )
    .bind(command_key.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        Some((stored_operation, aggregate_id, secondary_id)) if stored_operation == operation => {
            Ok(Some((aggregate_id, secondary_id)))
        }
        Some(_) => Err(SecretServiceError::IdempotencyConflict),
        None => Ok(None),
    }
}

async fn record_command(
    tx: &mut Transaction<'_, Postgres>,
    command_key: SecretCommandKey,
    operation: &str,
    aggregate_id: Uuid,
    secondary_id: Option<Uuid>,
    identity: &AuthenticatedIdentity,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO secret_command_inbox
         (command_key, operation, aggregate_id, secondary_id,
          requester_id, request_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(command_key.as_bytes().as_slice())
    .bind(operation)
    .bind(aggregate_id)
    .bind(secondary_id)
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// Audit fields stay explicit so values cannot be hidden in an untyped payload.
#[allow(clippy::too_many_arguments)]
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    owner_organization_id: Uuid,
    operation: &str,
    permission: &str,
    secret_id: Option<SecretId>,
    version_id: Option<SecretVersionId>,
    grant_id: Option<SecretGrantId>,
    import_id: Option<SecretImportId>,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO secret_audit_events
         (id, owner_organization_id, requester_id, secret_id,
          secret_version_id, grant_id, import_id, operation, permission,
          target_kind, target_id, delivery_mode, decision, outcome,
          request_id, command_id, authorization_model_version, policy_version)
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9,
                COALESCE(secret_import.target_kind, secret_grant.target_kind),
                COALESCE(secret_import.target_id, secret_grant.target_id),
                CASE
                    WHEN $8 IN ('bind_raw', 'receive_raw') THEN 'raw'
                    WHEN $8 IN ('bind_brokered', 'use_brokered') THEN 'brokered'
                    ELSE NULL
                END,
                'allow', $10, $11, $11, $12, 'command/v1'
         FROM (SELECT 1) AS singleton
         LEFT JOIN secret_grants AS secret_grant ON secret_grant.id = $6
         LEFT JOIN secret_imports AS secret_import ON secret_import.id = $7",
    )
    .bind(Uuid::new_v4())
    .bind(owner_organization_id)
    .bind(identity.user_id.as_uuid())
    .bind(secret_id.map(SecretId::as_uuid))
    .bind(version_id.map(SecretVersionId::as_uuid))
    .bind(grant_id.map(SecretGrantId::as_uuid))
    .bind(import_id.map(SecretImportId::as_uuid))
    .bind(operation)
    .bind(permission)
    .bind(outcome)
    .bind(identity.request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn append_event(
    tx: &mut Transaction<'_, Postgres>,
    aggregate_id: Uuid,
    subject: &str,
    event_type: &str,
    payload: serde_json::Value,
) -> Result<(), SecretServiceError> {
    sqlx::query(
        "INSERT INTO outbox
         (id, aggregate_type, aggregate_id, subject, event_type,
          payload, occurred_at)
         VALUES ($1, 'secret', $2, $3, $4, $5, now())",
    )
    .bind(Uuid::new_v4())
    .bind(aggregate_id)
    .bind(subject)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Non-sensitive secret command failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretServiceError {
    /// Authorization denied the exact operation.
    #[error("secret command is not authorized")]
    AuthorizationDenied,
    /// Secret/grant/target is missing, hidden, revoked, or expired.
    #[error("secret authority is unavailable")]
    Unavailable,
    /// A target belongs to another organization.
    #[error("secret delegation cannot cross organization boundaries")]
    CrossOrganization,
    /// An accepted target differs from the source grant.
    #[error("secret grant target does not match the accepting target")]
    TargetMismatch,
    /// Delivery mode ceiling is empty or malformed.
    #[error("secret delivery modes are invalid")]
    InvalidDeliveryModes,
    /// Expected active version lost a compare-and-swap race.
    #[error("secret active version changed concurrently")]
    StaleActiveVersion,
    /// Version sequence cannot be represented.
    #[error("secret version sequence is exhausted")]
    VersionSequenceExhausted,
    /// Purge was attempted while usable runtime leases exist.
    #[error("secret encrypted material is retained by active leases")]
    ActiveLeases,
    /// Lifecycle does not permit this operation.
    #[error("secret lifecycle does not permit this operation")]
    InvalidLifecycle,
    /// One command key was reused for another operation.
    #[error("secret command idempotency identity conflicts")]
    IdempotencyConflict,
    /// Stored idempotency result is incomplete.
    #[error("secret command idempotency record is invalid")]
    CorruptIdempotencyRecord,
    /// Stored ownership violates the database invariant.
    #[error("stored secret owner is invalid")]
    InvalidStoredData,
    /// Release does not declare the selected symbolic slot.
    #[error("release does not declare the selected secret slot")]
    SlotNotDeclared,
    /// Requested mode, phase, or destination exceeds a declaration or grant.
    #[error("secret binding policy exceeds its declaration or grant")]
    BindingPolicyMismatch,
    /// Selected attachments or instance fall outside the import target.
    #[error("secret binding is outside the import target scope")]
    BindingOutOfScope,
    /// Active instance revision changed during configuration.
    #[error("agent instance revision changed concurrently")]
    StaleInstanceRevision,
    /// Runtime authority must be short-lived and expire in the future.
    #[error("secret runtime lease lifetime is invalid")]
    InvalidLeaseLifetime,
    /// Bearer authority was already issued and cannot be replayed.
    #[error("secret runtime credential was already issued")]
    CredentialAlreadyIssued,
    /// Presented runtime credential is unknown, expired, or run-mismatched.
    #[error("secret runtime authentication is denied")]
    RuntimeAuthenticationDenied,
    /// Broker request exceeds its exact semantic capability.
    #[error("secret broker request is denied")]
    BrokerRequestDenied,
    /// Sanitized broker response exceeded the configured bound.
    #[error("secret broker response is too large")]
    BrokerResponseTooLarge,
    /// Application adapter rejected or could not complete the request.
    #[error(transparent)]
    BrokerAdapter(#[from] BrokerAdapterError),
    /// JSON normalization failed.
    #[error("secret binding policy serialization failed")]
    Serialization(#[from] serde_json::Error),
    /// Authorization provider failure.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// Domain validation failure.
    #[error(transparent)]
    Domain(#[from] secret_domain::SecretValueError),
    /// Encryption/key-provider failure.
    #[error(transparent)]
    Encryption(#[from] SecretStoreError),
    /// Database failure.
    #[error("secret persistence failed")]
    Database(#[from] sqlx::Error),
}
