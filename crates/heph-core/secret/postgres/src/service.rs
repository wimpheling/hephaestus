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
#[path = "service/binding_persist.rs"]
mod binding_persist;
#[path = "service/binding_prepare.rs"]
mod binding_prepare;
#[path = "service/broker_authorization.rs"]
mod broker_authorization;
#[path = "service/broker_commands.rs"]
mod broker_commands;
#[path = "service/broker_execution.rs"]
mod broker_execution;
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
    publication_repository_binding_id: Option<Uuid>,
    secret_slot_schema: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct CarriedCapabilityRow {
    source_binding_id: Uuid,
    requirement_id: Uuid,
    slot_key: String,
    requirement_resource_kind: String,
    required_operations: Vec<String>,
    optional_operations: Vec<String>,
    slot_required: bool,
    resource_kind: String,
    resource_id: Uuid,
    granted_operations: Vec<String>,
    authorization_model_version: String,
}

fn clone_capability_binding(
    row: &CarriedCapabilityRow,
) -> Result<CapabilityBinding, SecretServiceError> {
    let requirement_kind = parse_capability_resource_kind(&row.requirement_resource_kind)?;
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(row.requirement_id),
        CapabilitySlotKey::parse(row.slot_key.clone())
            .map_err(|_| SecretServiceError::InvalidStoredData)?,
        requirement_kind,
        row.required_operations
            .iter()
            .map(|operation| parse_capability_operation(operation))
            .collect::<Result<Vec<_>, _>>()?,
        row.optional_operations
            .iter()
            .map(|operation| parse_capability_operation(operation))
            .collect::<Result<Vec<_>, _>>()?,
        row.slot_required,
    )
    .map_err(|_| SecretServiceError::InvalidStoredData)?;
    CapabilityBinding::bind(
        CapabilityBindingId::new(),
        &requirement,
        CapabilityResource::new(
            parse_capability_resource_kind(&row.resource_kind)?,
            row.resource_id,
        ),
        row.granted_operations
            .iter()
            .map(|operation| parse_capability_operation(operation))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|_| SecretServiceError::InvalidStoredData)
}

fn parse_capability_resource_kind(
    value: &str,
) -> Result<CapabilityResourceKind, SecretServiceError> {
    match value {
        "repository" => Ok(CapabilityResourceKind::Repository),
        "project" => Ok(CapabilityResourceKind::Project),
        "agent_instance" => Ok(CapabilityResourceKind::AgentInstance),
        "gateway" => Ok(CapabilityResourceKind::Gateway),
        "run" => Ok(CapabilityResourceKind::Run),
        "state_volume" => Ok(CapabilityResourceKind::StateVolume),
        _ => Err(SecretServiceError::InvalidStoredData),
    }
}

fn parse_capability_operation(value: &str) -> Result<CapabilityOperation, SecretServiceError> {
    match value {
        "inspect" => Ok(CapabilityOperation::Inspect),
        "configure" => Ok(CapabilityOperation::Configure),
        "execute" => Ok(CapabilityOperation::Execute),
        "update" => Ok(CapabilityOperation::Update),
        "pause" => Ok(CapabilityOperation::Pause),
        "recover" => Ok(CapabilityOperation::Recover),
        "cancel" => Ok(CapabilityOperation::Cancel),
        "attach" => Ok(CapabilityOperation::Attach),
        "restore" => Ok(CapabilityOperation::Restore),
        "git_read" => Ok(CapabilityOperation::GitRead),
        "create_ref" => Ok(CapabilityOperation::CreateRef),
        "update_ref" => Ok(CapabilityOperation::UpdateRef),
        "force_update_ref" => Ok(CapabilityOperation::ForceUpdateRef),
        "delete_ref" => Ok(CapabilityOperation::DeleteRef),
        "create_tag" => Ok(CapabilityOperation::CreateTag),
        "delete_tag" => Ok(CapabilityOperation::DeleteTag),
        "trigger_run" => Ok(CapabilityOperation::TriggerRun),
        "manage_attachments" => Ok(CapabilityOperation::ManageAttachments),
        _ => Err(SecretServiceError::InvalidStoredData),
    }
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
struct BrokeredRuleBindingRow {
    instance_id: Uuid,
    instance_revision_id: Uuid,
    import_id: Uuid,
    delivery_mode: String,
    destinations: Vec<String>,
    secret_id: Uuid,
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
struct BrokeredSecretDenialContextRow {
    session_id: Uuid,
    run_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    attachment_id: Option<Uuid>,
    phase: String,
    expires_at: OffsetDateTime,
    lease_id: Uuid,
    secret_version_id: Uuid,
    destinations: Vec<String>,
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

#[derive(sqlx::FromRow)]
struct GatewayEncryptedVersionRow {
    header_name: String,
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
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
    let mut tx = resolver_pool
        .begin()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?
    .ok_or(SecretServiceError::Unavailable)?;
    if raw_observed {
        sqlx::query(
            "UPDATE secret_leases
               SET raw_material_observed = true
               WHERE id = $1 AND status = 'active'",
        )
        .bind(lease.lease_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    }
    record_runtime_use_tx(&mut tx, session, lease, mode, outcome)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    tx.commit()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
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

fn gateway_encrypted_version(
    row: GatewayEncryptedVersionRow,
) -> Result<(VersionContext, EncryptedSecretVersion), GatewayError> {
    let owner = match (row.organization_id, row.project_id) {
        (Some(id), None) => SecretOwner::Organization(OrganizationId::from_uuid(id)),
        (None, Some(id)) => SecretOwner::Project(ProjectId::from_uuid(id)),
        _ => return Err(GatewayError::InvalidInboundSecretRule),
    };
    let version_id = SecretVersionId::from_uuid(row.version_id);
    let context = VersionContext {
        owner,
        secret_id: SecretId::from_uuid(row.secret_id),
        version_id,
        sequence: u64::try_from(row.sequence)
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        media_type: String::from("application/octet-stream"),
    };
    let encrypted = EncryptedSecretVersion {
        version_id,
        algorithm: row.algorithm,
        key_reference: row.key_reference,
        data_nonce: row
            .data_nonce
            .try_into()
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        ciphertext: row.ciphertext,
        wrap_nonce: row
            .wrap_nonce
            .try_into()
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        wrapped_data_key: row.wrapped_data_key,
        associated_data_hash: row
            .associated_data_hash
            .try_into()
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
        content_length: u32::try_from(row.content_length)
            .map_err(|_| GatewayError::InvalidInboundSecretRule)?,
    };
    Ok((context, encrypted))
}

async fn record_runtime_use(
    resolver_pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    let mut tx = resolver_pool
        .begin()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    record_runtime_use_tx(&mut tx, session, lease, DeliveryMode::Brokered, outcome)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    tx.commit()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}

// The decision commits before the external call, and its outcome commits after.
// A decision without an outcome honestly represents an interrupted operation.
#[allow(clippy::too_many_arguments)] // Keep the value-free audit fields explicit at this boundary.
async fn record_https_operation(
    pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    request_id: Uuid,
    rule_id: Option<Uuid>,
    decision: Option<&str>,
    outcome: Option<&str>,
    reason_code: Option<&str>,
) -> Result<(), SecretServiceError> {
    let inserted = sqlx::query(
        "INSERT INTO brokered_secret_audit_events
         (id, lease_snapshot_id, rule_id, runtime_session_id, run_id,
          request_id, event_kind, decision, outcome, reason_code, occurred_at)
         SELECT $1, snapshot.id, snapshot.rule_id, snapshot.runtime_session_id,
                snapshot.run_id, $2, $3, $4, $5, $6, now()
         FROM brokered_secret_lease_snapshots snapshot
         WHERE snapshot.lease_id = $7 AND snapshot.runtime_session_id = $8
           AND snapshot.run_id = $9 AND snapshot.rule_id = $10",
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(if decision.is_some() {
        "authorization_decision"
    } else {
        "substitution_use"
    })
    .bind(decision)
    .bind(outcome)
    .bind(reason_code)
    .bind(lease.lease_id)
    .bind(session.session_id)
    .bind(session.run_id)
    .bind(rule_id)
    .execute(pool)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    if (decision == Some("allow") || outcome.is_some()) && inserted.rows_affected() != 1 {
        return Err(SecretServiceError::Persistence);
    }
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
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
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

fn broker_request_rule_id(request: &BrokerRequest) -> Option<Uuid> {
    serde_json::from_slice::<serde_json::Value>(&request.body)
        .ok()
        .and_then(|value| {
            value
                .get("rule_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
        })
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
