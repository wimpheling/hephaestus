//! `PostgreSQL` implementation of encrypted secret commands and runtime resolution.
#![allow(clippy::wildcard_imports)] // Keep adapter method signatures aligned with the port.

use async_trait::async_trait;
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{
    AUTHORIZATION_MODEL_VERSION, PostgresMelangeAuthorizer, audit_decision,
    begin_actor_transaction, begin_runtime_transaction,
};
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName as BrokeredHeaderName,
    HttpInjectionLocation,
};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
};
use forge_domain::{CommitSha, GitRef, ProjectId};
use gateway_domain::{GatewayError, GatewayInboundSecretResolver, InboundGatewaySecretRule};
use heph_secret::SecretMountProvider;
use http::{HeaderName, HeaderValue};
use identity_domain::{AuthenticatedIdentity, OrganizationId};
use release_domain::{AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use secret_application::*;
use secret_domain::*;
use secret_store::{EncryptedSecretVersion, EncryptedStore, KeyProvider, VersionContext};
use serde_json::json;
use sha2::Digest;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

#[path = "service/binding_commands.rs"]
mod binding_commands;
#[path = "service/binding_models.rs"]
mod binding_models;
#[path = "service/binding_persist.rs"]
mod binding_persist;
#[path = "service/binding_policy.rs"]
mod binding_policy;
#[path = "service/binding_prepare.rs"]
mod binding_prepare;
#[path = "service/broker_authorization.rs"]
mod broker_authorization;
#[path = "service/broker_commands.rs"]
mod broker_commands;
#[path = "service/broker_execution.rs"]
mod broker_execution;
#[path = "service/broker_usage.rs"]
mod broker_usage;
#[path = "service/dispatch_leases.rs"]
mod dispatch_leases;
#[path = "service/dispatch_resolution.rs"]
mod dispatch_resolution;
#[path = "service/grant_accept.rs"]
mod grant_accept;
#[path = "service/grant_commands.rs"]
mod grant_commands;
#[path = "service/lifecycle_commands.rs"]
mod lifecycle_commands;
#[path = "service/runtime_receive.rs"]
mod runtime_receive;
#[path = "service/runtime_versions.rs"]
mod runtime_versions;

use binding_models::{
    BrokeredRuleBindingRow, CarriedBindingRow, CarriedCapabilityRow, EligibleImportRow,
    RevisionCloneRow, clone_capability_binding,
};
use binding_policy::{
    declared_slot, insert_binding_copy, load_eligible_import, unresolved_required_diagnostics,
    validate_binding_scope, validate_carried_policy, validate_declared_binding,
    validate_import_policy,
};
use broker_usage::{
    broker_request_rule_id, record_https_operation, record_runtime_use, record_runtime_use_tx,
    validate_broker_request,
};
use runtime_versions::{
    BrokeredSecretDenialContextRow, GatewayEncryptedVersionRow, RuntimeLeaseAuthorizationRow,
    RuntimeSessionRow, gateway_encrypted_version, load_runtime_version,
};
#[path = "service/secret_commands.rs"]
mod secret_commands;

/// PostgreSQL-backed encrypted secret command service.
#[derive(Clone)]
pub struct SecretService<K> {
    pool: PgPool,
    encrypted_store: EncryptedStore<K>,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

/// Agent-facing runtime service with a non-bypass authorization pool and a
/// distinct narrow worker pool for exact ciphertext resolution.
#[derive(Clone)]
pub struct SecretRuntimeService<K> {
    authorization_pool: PgPool,
    resolver_pool: PgPool,
    encrypted_store: EncryptedStore<K>,
    authorizer: Arc<PostgresMelangeAuthorizer>,
    mount_provider: Option<Arc<dyn SecretMountProvider>>,
}

/// Host-only resolver for declared inbound gateway webhook-secret rules.
/// It decrypts an exact live invocation lease only long enough to build the
/// edge's non-serializable constant-time matcher.
#[derive(Clone)]
pub struct GatewayIngressSecretResolver<K> {
    resolver_pool: PgPool,
    encrypted_store: EncryptedStore<K>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreAdapterDenialClass {
    AuthenticationDenied,
    AuthorityUnavailable,
    AuthorizationDenied,
    RequestDenied,
    PersistenceFailure,
    SecretResolutionFailure,
    OtherFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreAdapterDenialStage {
    SessionAuthentication,
    LeaseAuthorization,
    RequestAuthorization,
    VersionLoading,
    Decryption,
}

#[derive(Debug, sqlx::FromRow)]
struct BrokeredHttpsRuleSnapshotRow {
    rule_id: Uuid,
    destination_origin: String,
    header_name: String,
    header_prefix: Option<String>,
}

impl PreAdapterDenialStage {
    const fn as_str(self) -> &'static str {
        match self {
            Self::SessionAuthentication => "session-authentication",
            Self::LeaseAuthorization => "lease-authorization",
            Self::RequestAuthorization => "request-authorization",
            Self::VersionLoading => "version-loading",
            Self::Decryption => "decryption",
        }
    }
}

impl PreAdapterDenialClass {
    const fn from_error(error: &SecretServiceError) -> Self {
        match error {
            SecretServiceError::RuntimeAuthenticationDenied => Self::AuthenticationDenied,
            SecretServiceError::Unavailable => Self::AuthorityUnavailable,
            SecretServiceError::AuthorizationDenied => Self::AuthorizationDenied,
            SecretServiceError::BrokerRequestDenied => Self::RequestDenied,
            SecretServiceError::Persistence => Self::PersistenceFailure,
            SecretServiceError::Encryption(_) => Self::SecretResolutionFailure,
            _ => Self::OtherFailure,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::AuthenticationDenied => "authentication_denied",
            Self::AuthorityUnavailable => "authority_unavailable",
            Self::AuthorizationDenied => "authorization_denied",
            Self::RequestDenied => "request_denied",
            Self::PersistenceFailure => "persistence_failure",
            Self::SecretResolutionFailure => "secret_resolution_failure",
            Self::OtherFailure => "other_failure",
        }
    }
}

impl<K: KeyProvider + Send + Sync> GatewayIngressSecretResolver<K> {
    /// Creates the resolver over the narrow worker pool and host KMS provider.
    #[must_use]
    pub const fn new(resolver_pool: PgPool, encrypted_store: EncryptedStore<K>) -> Self {
        Self {
            resolver_pool,
            encrypted_store,
        }
    }
}

#[async_trait]
impl<K: KeyProvider + Send + Sync> GatewayInboundSecretResolver
    for GatewayIngressSecretResolver<K>
{
    async fn rules_for_invocation(
        &self,
        invocation_id: Uuid,
        route_id: Uuid,
        gateway_revision_id: Uuid,
    ) -> Result<Vec<InboundGatewaySecretRule>, GatewayError> {
        let mut transaction = self
            .resolver_pool
            .begin()
            .await
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        let rows = sqlx::query_as::<_, GatewayEncryptedVersionRow>(
            "SELECT rule.header_name,
                    secret.id AS secret_id, version.id AS version_id, version.sequence,
                    secret.organization_id, secret.project_id,
                    version.algorithm, version.key_reference, version.data_nonce,
                    version.ciphertext, version.wrap_nonce, version.wrapped_data_key,
                    version.associated_data_hash, version.content_length
             FROM gateway_secret_leases AS lease
             JOIN gateway_runtime_authority_sessions AS session
               ON session.id = lease.runtime_session_id
              AND session.invocation_id = lease.invocation_id
             JOIN gateway_invocations AS invocation ON invocation.id = lease.invocation_id
             JOIN gateway_brokered_secret_rules AS rule ON rule.id = lease.rule_id
             JOIN gateway_secret_bindings AS binding ON binding.id = lease.binding_id
             JOIN secret_versions AS version ON version.id = lease.secret_version_id
             JOIN secrets AS secret ON secret.id = version.secret_id
             JOIN secret_imports AS imported ON imported.id = binding.import_id
             JOIN secret_grants AS granted ON granted.id = imported.grant_id
             WHERE lease.invocation_id = $1
               AND rule.gateway_route_id = $2
               AND invocation.gateway_revision_id = $3
               AND invocation.outcome = 'accepted'
               AND rule.gateway_revision_id = invocation.gateway_revision_id
               AND binding.gateway_revision_id = invocation.gateway_revision_id
               AND lease.secret_version_id = binding.secret_version_id
               AND lease.status = 'active' AND lease.expires_at > now()
               AND session.status IN ('pending_handoff', 'active') AND session.expires_at > now()
               AND binding.status = 'active' AND imported.status = 'active'
               AND granted.status = 'active' AND secret.status = 'active'
               AND version.status = 'active' AND version.revoked_at IS NULL
               AND version.purged_at IS NULL",
        )
        .bind(invocation_id)
        .bind(route_id)
        .bind(gateway_revision_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        transaction
            .commit()
            .await
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
        rows.into_iter()
            .map(|row| {
                let header = HeaderName::from_bytes(row.header_name.as_bytes())
                    .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
                let (context, encrypted) = gateway_encrypted_version(row)?;
                let value = self
                    .encrypted_store
                    .resolve(&context, &encrypted)
                    .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
                let placeholder =
                    HeaderValue::from_str(&format!("heph-placeholder:v1:{}", context.version_id))
                        .map_err(|_| GatewayError::InvalidInboundSecretRule)?;
                InboundGatewaySecretRule::new(header, value.expose().to_vec(), placeholder)
            })
            .collect()
    }
}

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Creates a service with explicit encryption and authorization providers.
    #[must_use]
    pub const fn new(
        pool: PgPool,
        encrypted_store: EncryptedStore<K>,
        authorizer: Arc<PostgresMelangeAuthorizer>,
    ) -> Self {
        Self {
            pool,
            encrypted_store,
            authorizer,
        }
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
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // Keep rejected attempts durable while allowing the caller's
            // command transaction to roll back all domain changes.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity)
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                permission,
                object,
                decision,
                identity.request_id,
            )
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
            audit_tx
                .commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
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
                    .await
                    .map_err(|_| SecretServiceError::Persistence)?
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
            .await
            .map_err(|_| SecretServiceError::Persistence)?
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
                    .await
                    .map_err(|_| SecretServiceError::Persistence)?;
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
                    .await
                    .map_err(|_| SecretServiceError::Persistence)?
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}

#[async_trait]
impl<K: KeyProvider + Send + Sync> SecretCommandService for SecretService<K> {
    async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateSecret,
    ) -> Result<CreatedSecret, SecretServiceError> {
        self.create(identity, command).await
    }
    async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotateSecret,
    ) -> Result<SecretVersionId, SecretServiceError> {
        self.rotate(identity, command).await
    }
    async fn grant(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantSecret,
    ) -> Result<SecretGrantId, SecretServiceError> {
        self.grant(identity, command).await
    }
    async fn accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        command: AcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        self.accept_import(identity, command).await
    }
    async fn grant_and_accept(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantAndAcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        self.grant_and_accept(identity, command).await
    }
    async fn bind(
        &self,
        identity: &AuthenticatedIdentity,
        command: BindSecret,
    ) -> Result<AgentInstanceRevisionId, SecretServiceError> {
        self.bind(identity, command).await
    }
    async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        self.revoke(identity, secret_id).await
    }
    async fn set_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
        enabled: bool,
    ) -> Result<(), SecretServiceError> {
        self.set_enabled(identity, secret_id, enabled).await
    }
    async fn purge(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        self.purge(identity, secret_id).await
    }
}
#[async_trait]
impl<K: KeyProvider + Send + Sync> SecretDispatchResolver for SecretService<K> {
    async fn resolve_for_dispatch(
        &self,
        identity: &AuthenticatedIdentity,
        command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        self.resolve_for_dispatch(identity, command).await
    }
}
#[async_trait]
impl<K: KeyProvider + Send + Sync> SecretRuntimeResolver for SecretRuntimeService<K> {
    async fn receive_raw(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        run_id: RunId,
        slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        self.receive_raw(credential, run_id, slot).await
    }
    async fn use_brokered(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        adapter: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError> {
        self.use_brokered(credential, request, adapter).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PostgresMelangeAuthorizer, PreAdapterDenialClass, PreAdapterDenialStage,
        SecretRuntimeService, SecretServiceError,
    };
    use heph_secret::{
        EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretMountProvider,
        SecretRuntimeError,
    };
    use secret_store::{EncryptedStore, LocalKeyProvider, SecretStoreError};
    use sqlx::postgres::PgPoolOptions;
    use std::{collections::BTreeSet, sync::Arc};
    use uuid::Uuid;

    #[derive(Debug)]
    struct TestMountProvider;

    impl SecretMountProvider for TestMountProvider {
        fn validate_config(
            &self,
            _config: &EphemeralSecretConfig,
        ) -> Result<(), SecretRuntimeError> {
            Ok(())
        }

        fn materialize(
            &self,
            _config: &EphemeralSecretConfig,
            _run_id: runtime_types::RunId,
            _files: Vec<RawSecretFile>,
            _credential: Option<&secret_domain::OpaqueRuntimeCredential>,
        ) -> Result<MaterializedSecretMount, SecretRuntimeError> {
            Err(SecretRuntimeError::InvalidRoot)
        }

        fn discard_materialized(
            &self,
            _config: &EphemeralSecretConfig,
            _opaque_directory: Uuid,
        ) -> Result<(), SecretRuntimeError> {
            Ok(())
        }

        fn destroy_confirmed(
            &self,
            _config: &EphemeralSecretConfig,
            _opaque_directory: Uuid,
        ) -> Result<(), SecretRuntimeError> {
            Ok(())
        }

        fn reconcile_orphans(
            &self,
            _config: &EphemeralSecretConfig,
            _live_directories: &BTreeSet<String>,
        ) -> Result<usize, SecretRuntimeError> {
            Ok(0)
        }
    }

    fn runtime_service() -> SecretRuntimeService<LocalKeyProvider> {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://localhost/hephaestus")
            .expect("lazy PostgreSQL pool");
        let keys =
            LocalKeyProvider::new("test/v1", [("test/v1", [7_u8; 32])]).expect("test key provider");
        SecretRuntimeService::new(
            pool.clone(),
            pool,
            EncryptedStore::new(keys),
            Arc::new(PostgresMelangeAuthorizer),
        )
    }

    #[tokio::test]
    async fn mount_provider_builder_installs_provider() {
        let runtime = runtime_service().with_mount_provider(Arc::new(TestMountProvider));
        assert!(runtime.mount_provider().is_some());
    }

    #[test]
    fn pre_adapter_denial_classes_are_distinct_and_payload_free() {
        let cases = [
            (
                SecretServiceError::RuntimeAuthenticationDenied,
                "authentication_denied",
            ),
            (SecretServiceError::Unavailable, "authority_unavailable"),
            (
                SecretServiceError::AuthorizationDenied,
                "authorization_denied",
            ),
            (SecretServiceError::BrokerRequestDenied, "request_denied"),
            (SecretServiceError::Persistence, "persistence_failure"),
            (
                SecretServiceError::Encryption(SecretStoreError::Authentication),
                "secret_resolution_failure",
            ),
            (SecretServiceError::InvalidLifecycle, "other_failure"),
        ];
        let mut classes = std::collections::BTreeSet::new();
        for (error, expected) in cases {
            let class = PreAdapterDenialClass::from_error(&error);
            assert_eq!(class.as_str(), expected);
            assert!(classes.insert(class.as_str()));
            assert!(!class.as_str().contains("sentinel"));
            assert!(!class.as_str().contains("password"));
        }
    }

    #[test]
    fn pre_adapter_denial_stages_are_static_and_distinct() {
        let stages = [
            PreAdapterDenialStage::SessionAuthentication,
            PreAdapterDenialStage::LeaseAuthorization,
            PreAdapterDenialStage::RequestAuthorization,
            PreAdapterDenialStage::VersionLoading,
            PreAdapterDenialStage::Decryption,
        ];
        let values = stages.map(PreAdapterDenialStage::as_str);
        assert_eq!(
            values,
            [
                "session-authentication",
                "lease-authorization",
                "request-authorization",
                "version-loading",
                "decryption",
            ]
        );
    }
}
