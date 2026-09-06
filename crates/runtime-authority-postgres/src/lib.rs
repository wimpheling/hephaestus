//! `PostgreSQL` persistence for immutable runtime authority snapshots and
//! hash-only short-lived sessions.

use async_trait::async_trait;
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, CapabilityBinding, CapabilityBindingId,
    CapabilityOperation, CapabilityRequirement, CapabilityRequirementId, CapabilityResource,
    CapabilityResourceKind, CapabilitySlotKey, RuntimeCredentialGeneration, RuntimeInvocation,
    RuntimeSessionId, RuntimeSessionStatus, WorkloadKind, WorkloadPrincipal,
};
use run_domain::Run;
use runtime_authority::{
    GatewayRuntimeAuthorityIssuer, GatewayRuntimeSessionRequest, NewRuntimeSession,
    RuntimeAuthorityError, RuntimeHandoffStore, RuntimeSessionIssuer, RuntimeSessionRepository,
    StoredRuntimeSession,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

/// `PostgreSQL` runtime authority repository for trusted workers.
#[derive(Clone)]
pub struct PgRuntimeSessionRepository {
    pool: PgPool,
}

impl PgRuntimeSessionRepository {
    /// Creates a repository using a worker-role connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Resolves the exact immutable instance-revision binding ceiling for a
    /// dispatchable run.
    ///
    /// # Errors
    ///
    /// Fails closed for stale run lifecycle, incomplete required bindings,
    /// malformed persisted capability data, or storage failure.
    pub async fn resolve_snapshot(
        &self,
        run: &Run,
        authorization_model_version: &str,
    ) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
        let dispatchable: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM runs AS run
                JOIN agent_instances AS instance ON instance.id = run.instance_id
                JOIN agent_instance_revisions AS revision
                  ON revision.id = run.instance_revision_id
                 AND revision.instance_id = run.instance_id
                JOIN release_agents AS release_agent
                  ON release_agent.id = revision.release_agent_id
                JOIN releases AS release ON release.id = release_agent.release_id
                WHERE run.id = $1
                  AND run.instance_id = $2
                  AND run.instance_revision_id = $3
                  AND run.state = 'provisioning'
                  AND (
                      (run.run_kind = 'update'
                          AND instance.state = 'updating'
                          AND EXISTS (
                              SELECT 1
                              FROM agent_updates AS update
                              WHERE update.hook_run_id = run.id
                                AND update.instance_id = run.instance_id
                                AND update.candidate_revision_id = revision.id
                                AND update.state = 'hook_running'
                          ))
                      OR (run.run_kind = 'normal'
                          AND instance.state IN ('active', 'update_rejected')
                          AND instance.active_revision_id = revision.id)
                  )
                  AND revision.runnable
                  AND release.state = 'published'
                  AND NOT EXISTS (
                      SELECT 1
                      FROM release_capability_requirements AS requirement
                      WHERE requirement.release_agent_id = revision.release_agent_id
                        AND requirement.slot_required
                        AND NOT EXISTS (
                            SELECT 1 FROM agent_capability_bindings AS binding
                            WHERE binding.instance_revision_id = revision.id
                              AND binding.requirement_id = requirement.id
                        )
                  )
            )",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        if !dispatchable {
            return Err(RuntimeAuthorityError::Persistence);
        }

        let rows = sqlx::query_as::<_, SnapshotBindingRow>(
            "SELECT binding.id AS binding_id, requirement.id AS requirement_id,
                    requirement.slot_key, requirement.resource_kind,
                    requirement.required_operations,
                    requirement.optional_operations, requirement.slot_required,
                    requirement.normalized_hash AS requirement_hash,
                    binding.resource_id, binding.granted_operations,
                    binding.normalized_hash AS binding_hash,
                    binding.authorization_model_version
             FROM agent_capability_bindings AS binding
             JOIN release_capability_requirements AS requirement
               ON requirement.id = binding.requirement_id
              AND requirement.release_agent_id = binding.release_agent_id
             WHERE binding.instance_revision_id = $1
             ORDER BY binding.slot_key, binding.id",
        )
        .bind(run.instance_revision_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        let mut bindings = Vec::with_capacity(rows.len());
        for row in rows {
            bindings.push(stored_binding(row, authorization_model_version)?);
        }
        AuthorizationSnapshot::new(
            AuthorizationSnapshotId::from_uuid(run.id.as_uuid()),
            WorkloadPrincipal::new(
                WorkloadKind::AgentInstance,
                run.instance_id.as_uuid(),
                run.instance_revision_id.as_uuid(),
            ),
            authorization_model_version,
            bindings,
        )
        .map_err(|_| RuntimeAuthorityError::Persistence)
    }

    /// Rechecks every exact snapshotted operation against current Mélange
    /// authorization immediately before provisioning.
    ///
    /// # Errors
    ///
    /// Returns a safe persistence failure for storage errors or `false` when
    /// any operation is no longer authorized.
    pub async fn live_authorized(
        &self,
        run: &Run,
        snapshot: &AuthorizationSnapshot,
    ) -> Result<bool, RuntimeAuthorityError> {
        if snapshot.principal().id != run.instance_id.as_uuid()
            || snapshot.principal().revision_id != run.instance_revision_id.as_uuid()
        {
            return Ok(false);
        }
        for binding in snapshot.bindings() {
            for operation in binding.granted_operations() {
                let relation = format!("agent_{}", operation.as_str());
                let allowed: bool = sqlx::query_scalar(
                    "SELECT check_permission(
                        'agent_instance', $1::text, $2,
                        $3, $4::text
                    ) = 1",
                )
                .bind(run.instance_id.as_uuid())
                .bind(relation)
                .bind(binding.resource().kind.as_str())
                .bind(binding.resource().id)
                .fetch_one(&self.pool)
                .await
                .map_err(storage)?;
                if !allowed {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    async fn locked(
        transaction: &mut Transaction<'_, Postgres>,
        session_id: RuntimeSessionId,
    ) -> Result<SessionRow, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM runtime_authority_sessions
             WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
        .ok_or(RuntimeAuthorityError::NotFound)
    }
}

/// `PostgreSQL` storage for gateway-only runtime snapshots and sessions.
///
/// This deliberately implements the same generic session repository contract
/// as agent runs while targeting the gateway-specific immutable tables.
#[derive(Clone)]
pub struct PgGatewayRuntimeSessionRepository {
    pool: PgPool,
}

impl PgGatewayRuntimeSessionRepository {
    /// Creates a gateway session repository over a worker-role pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn locked(
        transaction: &mut Transaction<'_, Postgres>,
        session_id: RuntimeSessionId,
    ) -> Result<SessionRow, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM gateway_runtime_authority_sessions WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(storage)?
        .ok_or(RuntimeAuthorityError::NotFound)
    }
}

#[async_trait]
impl RuntimeSessionRepository for PgGatewayRuntimeSessionRepository {
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM gateway_runtime_authority_sessions WHERE id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn create(
        &self,
        session: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let principal = session.identity.principal();
        let (WorkloadKind::Gateway, RuntimeInvocation::Gateway(invocation_id)) =
            (principal.kind, session.identity.invocation())
        else {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        };
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query(
            "INSERT INTO gateway_authorization_snapshots
                (id, invocation_id, gateway_id, gateway_revision_id,
                 authorization_model_version, normalized_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session.snapshot.id().as_uuid())
        .bind(invocation_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.snapshot.authorization_model_version())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        insert_gateway_snapshot_bindings(&mut transaction, session.snapshot).await?;
        let generation = i64::try_from(session.generation.get()).map_err(storage)?;
        sqlx::query(
            "INSERT INTO gateway_runtime_authority_sessions
                (id, snapshot_id, invocation_id, gateway_id, gateway_revision_id,
                 identity_hash, snapshot_hash, issuance_generation, credential_hash,
                 status, issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                     'pending_handoff', $10, $11)",
        )
        .bind(session.identity.id().as_uuid())
        .bind(session.snapshot.id().as_uuid())
        .bind(invocation_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.identity.normalized_hash().as_bytes().as_slice())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .bind(generation)
        .bind(session.credential_hash.as_bytes().as_slice())
        .bind(session.identity.issued_at())
        .bind(session.identity.expires_at())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(StoredRuntimeSession {
            id: session.identity.id(),
            snapshot_id: session.snapshot.id(),
            identity_hash: session.identity.normalized_hash(),
            generation: session.generation,
            status: RuntimeSessionStatus::PendingHandoff,
            issued_at: session.identity.issued_at(),
            expires_at: session.identity.expires_at(),
            acknowledged_at: None,
            revoked_at: None,
        })
    }

    async fn acknowledge(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        acknowledged_at: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked(&mut transaction, session_id).await?;
        if generation_from_i64(current.issuance_generation)? != generation {
            return Err(RuntimeAuthorityError::GenerationMismatch);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff
                if acknowledged_at >= current.issued_at && acknowledged_at < current.expires_at =>
            {
                sqlx::query("UPDATE gateway_runtime_authority_sessions SET status = 'active', acknowledged_at = $2, updated_at = $2 WHERE id = $1")
                    .bind(session_id.as_uuid()).bind(acknowledged_at).execute(&mut *transaction).await.map_err(storage)?;
            }
            RuntimeSessionStatus::Active => {}
            RuntimeSessionStatus::PendingHandoff
            | RuntimeSessionStatus::Revoked
            | RuntimeSessionStatus::Expired => {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn revoke(
        &self,
        session_id: RuntimeSessionId,
        revoked_at: OffsetDateTime,
        reason: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        if reason.is_empty() || reason.len() > 256 {
            return Err(RuntimeAuthorityError::Persistence);
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked(&mut transaction, session_id).await?;
        if revoked_at < current.issued_at {
            return Err(RuntimeAuthorityError::Persistence);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff | RuntimeSessionStatus::Active => {
                sqlx::query("UPDATE gateway_runtime_authority_sessions SET status = 'revoked', revoked_at = $2, revocation_reason = $3, updated_at = $2 WHERE id = $1")
                    .bind(session_id.as_uuid()).bind(revoked_at).bind(reason).execute(&mut *transaction).await.map_err(storage)?;
            }
            RuntimeSessionStatus::Revoked => {}
            RuntimeSessionStatus::Expired => return Err(RuntimeAuthorityError::SessionNotPending),
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn expire(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        sqlx::query("UPDATE gateway_runtime_authority_sessions SET status = 'expired', updated_at = $1 WHERE status IN ('pending_handoff', 'active') AND expires_at <= $1")
            .bind(now).execute(&self.pool).await.map(|result| result.rows_affected()).map_err(storage)
    }
}

/// Gateway authority issuer built from the common session/handoff machinery.
pub struct PgGatewayRuntimeAuthorityIssuer<H> {
    issuer: RuntimeSessionIssuer<PgGatewayRuntimeSessionRepository, H>,
    pool: PgPool,
    authorization_model_version: String,
}

impl<H> PgGatewayRuntimeAuthorityIssuer<H>
where
    H: RuntimeHandoffStore,
{
    /// Constructs an issuer using the explicit authorization model version.
    #[must_use]
    pub fn new(pool: PgPool, handoff: H, authorization_model_version: impl Into<String>) -> Self {
        Self {
            issuer: RuntimeSessionIssuer::new(
                PgGatewayRuntimeSessionRepository::new(pool.clone()),
                handoff,
            ),
            pool,
            authorization_model_version: authorization_model_version.into(),
        }
    }
}

#[async_trait]
impl<H> GatewayRuntimeAuthorityIssuer for PgGatewayRuntimeAuthorityIssuer<H>
where
    H: RuntimeHandoffStore,
{
    async fn issue_gateway(
        &self,
        request: GatewayRuntimeSessionRequest,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let snapshot =
            gateway_snapshot(&self.pool, request, &self.authorization_model_version).await?;
        let identity = capability_domain::RuntimeSessionIdentity::new(
            RuntimeSessionId::from_uuid(request.invocation_id.as_uuid()),
            snapshot.principal(),
            RuntimeInvocation::Gateway(request.invocation_id),
            &snapshot,
            request.issued_at,
            request.expires_at,
        )
        .map_err(|_| RuntimeAuthorityError::Persistence)?;
        let issued = self
            .issuer
            .issue(&snapshot, &identity, None, request.issued_at)
            .await?;
        Ok(issued.session)
    }
}

async fn gateway_snapshot(
    pool: &PgPool,
    request: GatewayRuntimeSessionRequest,
    authorization_model_version: &str,
) -> Result<AuthorizationSnapshot, RuntimeAuthorityError> {
    let rows = sqlx::query_as::<_, GatewaySnapshotBindingRow>(
        "SELECT binding.id AS binding_id, binding_grant.id AS grant_id, binding.slot_key,
                binding.mailbox_id AS resource_id
         FROM gateway_mailbox_bindings AS binding
         JOIN gateway_mailbox_binding_grants AS binding_grant
           ON binding_grant.binding_id = binding.id
         WHERE binding.gateway_id = $1
           AND binding.gateway_revision_id = $2
           AND binding_grant.status = 'active'
         ORDER BY binding.slot_key, binding.id",
    )
    .bind(request.gateway_id)
    .bind(request.gateway_revision_id)
    .fetch_all(pool)
    .await
    .map_err(storage)?;
    let mut bindings = Vec::with_capacity(rows.len());
    for row in rows {
        bindings.push(gateway_mailbox_binding(row)?);
    }
    AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(request.invocation_id.as_uuid()),
        WorkloadPrincipal::new(
            WorkloadKind::Gateway,
            request.gateway_id,
            request.gateway_revision_id,
        ),
        authorization_model_version,
        bindings,
    )
    .map_err(|_| RuntimeAuthorityError::Persistence)
}

async fn insert_gateway_snapshot_bindings(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot: &AuthorizationSnapshot,
) -> Result<(), RuntimeAuthorityError> {
    let rows = sqlx::query_as::<_, GatewaySnapshotBindingRow>(
        "SELECT binding.id AS binding_id, binding_grant.id AS grant_id, binding.slot_key,
                binding.mailbox_id AS resource_id
         FROM gateway_mailbox_bindings AS binding
         JOIN gateway_mailbox_binding_grants AS binding_grant
           ON binding_grant.binding_id = binding.id
         WHERE binding.gateway_id = $1
           AND binding.gateway_revision_id = $2
           AND binding_grant.status = 'active'
         ORDER BY binding.slot_key, binding.id",
    )
    .bind(snapshot.principal().id)
    .bind(snapshot.principal().revision_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(storage)?;
    if rows.len() != snapshot.bindings().len() {
        return Err(RuntimeAuthorityError::Persistence);
    }
    for (ordinal, row) in rows.into_iter().enumerate() {
        let persisted = gateway_mailbox_binding(GatewaySnapshotBindingRow {
            binding_id: row.binding_id,
            grant_id: row.grant_id,
            slot_key: row.slot_key.clone(),
            resource_id: row.resource_id,
        })?;
        let expected = snapshot
            .bindings()
            .find(|binding| binding.id() == persisted.id())
            .filter(|binding| *binding == &persisted)
            .ok_or(RuntimeAuthorityError::Persistence)?;
        sqlx::query(
            "INSERT INTO gateway_authorization_snapshot_bindings
                (snapshot_id, gateway_revision_id, ordinal, binding_id, grant_id,
                 binding_hash, slot_key, resource_kind, resource_id,
                 granted_operations)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'mailbox', $8, ARRAY['publish']::text[])",
        )
        .bind(snapshot.id().as_uuid())
        .bind(snapshot.principal().revision_id)
        .bind(i32::try_from(ordinal).map_err(storage)?)
        .bind(row.binding_id)
        .bind(row.grant_id)
        .bind(expected.normalized_hash().as_bytes().as_slice())
        .bind(row.slot_key)
        .bind(row.resource_id)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
    }
    Ok(())
}

#[derive(FromRow)]
struct GatewaySnapshotBindingRow {
    binding_id: Uuid,
    grant_id: Uuid,
    slot_key: String,
    resource_id: Uuid,
}

fn gateway_mailbox_binding(
    row: GatewaySnapshotBindingRow,
) -> Result<CapabilityBinding, RuntimeAuthorityError> {
    // Gateway declarations only support this fixed mailbox/publish shape.
    // The immutable binding ID is also the stable synthetic requirement ID,
    // making the resulting authority hash reproducible from persisted facts.
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(row.binding_id),
        CapabilitySlotKey::parse(row.slot_key).map_err(storage)?,
        CapabilityResourceKind::Mailbox,
        [CapabilityOperation::Publish],
        [],
        true,
    )
    .map_err(storage)?;
    let binding = CapabilityBinding::bind(
        CapabilityBindingId::from_uuid(row.binding_id),
        &requirement,
        CapabilityResource::new(CapabilityResourceKind::Mailbox, row.resource_id),
        [CapabilityOperation::Publish],
    )
    .map_err(storage)?;
    // `grant_id` is deliberately selected with every snapshot row. It is
    // persisted alongside this binding below, rather than being trusted from
    // mutable current grant state when a gateway later publishes.
    let _ = row.grant_id;
    Ok(binding)
}

fn stored_binding(
    row: SnapshotBindingRow,
    authorization_model_version: &str,
) -> Result<CapabilityBinding, RuntimeAuthorityError> {
    if row.authorization_model_version != authorization_model_version {
        return Err(RuntimeAuthorityError::Persistence);
    }
    let kind = resource_kind(&row.resource_kind)?;
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(row.requirement_id),
        CapabilitySlotKey::parse(row.slot_key).map_err(storage)?,
        kind,
        operations(&row.required_operations)?,
        operations(&row.optional_operations)?,
        row.slot_required,
    )
    .map_err(storage)?;
    if row.requirement_hash.as_slice() != requirement.normalized_hash().as_bytes() {
        return Err(RuntimeAuthorityError::Persistence);
    }
    let binding = CapabilityBinding::bind(
        CapabilityBindingId::from_uuid(row.binding_id),
        &requirement,
        CapabilityResource::new(kind, row.resource_id),
        operations(&row.granted_operations)?,
    )
    .map_err(storage)?;
    if row.binding_hash.as_slice() != binding.normalized_hash().as_bytes() {
        return Err(RuntimeAuthorityError::Persistence);
    }
    Ok(binding)
}

#[derive(FromRow)]
struct SnapshotBindingRow {
    binding_id: Uuid,
    requirement_id: Uuid,
    slot_key: String,
    resource_kind: String,
    required_operations: Vec<String>,
    optional_operations: Vec<String>,
    slot_required: bool,
    requirement_hash: Vec<u8>,
    resource_id: Uuid,
    granted_operations: Vec<String>,
    binding_hash: Vec<u8>,
    authorization_model_version: String,
}

fn resource_kind(value: &str) -> Result<CapabilityResourceKind, RuntimeAuthorityError> {
    match value {
        "repository" => Ok(CapabilityResourceKind::Repository),
        "project" => Ok(CapabilityResourceKind::Project),
        "agent_instance" => Ok(CapabilityResourceKind::AgentInstance),
        "gateway" => Ok(CapabilityResourceKind::Gateway),
        "run" => Ok(CapabilityResourceKind::Run),
        "state_volume" => Ok(CapabilityResourceKind::StateVolume),
        "mailbox" => Ok(CapabilityResourceKind::Mailbox),
        _ => Err(RuntimeAuthorityError::Persistence),
    }
}

fn operations(values: &[String]) -> Result<Vec<CapabilityOperation>, RuntimeAuthorityError> {
    values
        .iter()
        .map(|value| match value.as_str() {
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
            "publish" => Ok(CapabilityOperation::Publish),
            _ => Err(RuntimeAuthorityError::Persistence),
        })
        .collect()
}

#[async_trait]
impl RuntimeSessionRepository for PgRuntimeSessionRepository {
    async fn find(
        &self,
        session_id: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        sqlx::query_as::<_, SessionRow>(
            "SELECT id, snapshot_id, identity_hash, issuance_generation,
                    status, issued_at, expires_at, acknowledged_at, revoked_at
             FROM runtime_authority_sessions WHERE id = $1",
        )
        .bind(session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn create(
        &self,
        session: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let principal = session.identity.principal();
        let (WorkloadKind::AgentInstance, RuntimeInvocation::Run(run_id)) =
            (principal.kind, session.identity.invocation())
        else {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        };
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query(
            "INSERT INTO run_authorization_snapshots
                (id, run_id, instance_id, instance_revision_id,
                 authorization_model_version, normalized_hash)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(session.snapshot.id().as_uuid())
        .bind(run_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.snapshot.authorization_model_version())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;

        for (ordinal, binding) in session.snapshot.bindings().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(storage)?;
            let operations = binding
                .granted_operations()
                .map(capability_domain::CapabilityOperation::as_str)
                .collect::<Vec<_>>();
            sqlx::query(
                "INSERT INTO run_authorization_snapshot_bindings
                    (snapshot_id, instance_revision_id, ordinal, binding_id,
                     binding_hash, slot_key, resource_kind, resource_id,
                     granted_operations)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(session.snapshot.id().as_uuid())
            .bind(principal.revision_id)
            .bind(ordinal)
            .bind(binding.id().as_uuid())
            .bind(binding.normalized_hash().as_bytes().as_slice())
            .bind(binding.slot().as_str())
            .bind(binding.resource().kind.as_str())
            .bind(binding.resource().id)
            .bind(&operations)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        }

        insert_runtime_git_snapshot(
            &mut transaction,
            session.snapshot.id().as_uuid(),
            principal.revision_id,
            run_id.as_uuid(),
        )
        .await?;

        let generation = i64::try_from(session.generation.get()).map_err(storage)?;
        sqlx::query(
            "INSERT INTO runtime_authority_sessions
                (id, snapshot_id, run_id, instance_id, instance_revision_id,
                 attachment_id, identity_hash, snapshot_hash,
                 issuance_generation, credential_hash, status, issued_at,
                 expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     'pending_handoff', $11, $12)",
        )
        .bind(session.identity.id().as_uuid())
        .bind(session.snapshot.id().as_uuid())
        .bind(run_id.as_uuid())
        .bind(principal.id)
        .bind(principal.revision_id)
        .bind(session.attachment_id)
        .bind(session.identity.normalized_hash().as_bytes().as_slice())
        .bind(session.snapshot.normalized_hash().as_bytes().as_slice())
        .bind(generation)
        .bind(session.credential_hash.as_bytes().as_slice())
        .bind(session.identity.issued_at())
        .bind(session.identity.expires_at())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(StoredRuntimeSession {
            id: session.identity.id(),
            snapshot_id: session.snapshot.id(),
            identity_hash: session.identity.normalized_hash(),
            generation: session.generation,
            status: RuntimeSessionStatus::PendingHandoff,
            issued_at: session.identity.issued_at(),
            expires_at: session.identity.expires_at(),
            acknowledged_at: None,
            revoked_at: None,
        })
    }

    async fn acknowledge(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        acknowledged_at: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked(&mut transaction, session_id).await?;
        let current_generation = generation_from_i64(current.issuance_generation)?;
        if current_generation != generation {
            return Err(RuntimeAuthorityError::GenerationMismatch);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff
                if acknowledged_at >= current.issued_at && acknowledged_at < current.expires_at =>
            {
                sqlx::query(
                    "UPDATE runtime_authority_sessions
                     SET status = 'active', acknowledged_at = $2,
                         updated_at = $2
                     WHERE id = $1",
                )
                .bind(session_id.as_uuid())
                .bind(acknowledged_at)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
            }
            RuntimeSessionStatus::Active => {}
            RuntimeSessionStatus::PendingHandoff
            | RuntimeSessionStatus::Revoked
            | RuntimeSessionStatus::Expired => {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn revoke(
        &self,
        session_id: RuntimeSessionId,
        revoked_at: OffsetDateTime,
        reason: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        if reason.is_empty() || reason.len() > 256 {
            return Err(RuntimeAuthorityError::Persistence);
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let current = Self::locked(&mut transaction, session_id).await?;
        if revoked_at < current.issued_at {
            return Err(RuntimeAuthorityError::Persistence);
        }
        match parse_status(&current.status)? {
            RuntimeSessionStatus::PendingHandoff | RuntimeSessionStatus::Active => {
                sqlx::query(
                    "UPDATE runtime_authority_sessions
                     SET status = 'revoked', revoked_at = $2,
                         revocation_reason = $3, updated_at = $2
                     WHERE id = $1",
                )
                .bind(session_id.as_uuid())
                .bind(revoked_at)
                .bind(reason)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
            }
            RuntimeSessionStatus::Revoked => {}
            RuntimeSessionStatus::Expired => {
                return Err(RuntimeAuthorityError::SessionNotPending);
            }
        }
        transaction.commit().await.map_err(storage)?;
        self.find(session_id)
            .await?
            .ok_or(RuntimeAuthorityError::Persistence)
    }

    async fn expire(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        sqlx::query(
            "UPDATE runtime_authority_sessions
             SET status = 'expired', updated_at = $1
             WHERE status IN ('pending_handoff', 'active')
               AND expires_at <= $1",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map(|result| result.rows_affected())
        .map_err(storage)
    }
}

// Runtime Git authority is copied only from the release-owned publication
// binding. The database trigger verifies the complete copy and trigger parent.
async fn insert_runtime_git_snapshot(
    transaction: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    revision_id: Uuid,
    run_id: Uuid,
) -> Result<(), RuntimeAuthorityError> {
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
            (snapshot_id, instance_revision_id, binding_id, repository_id,
             grammar_version, git_operations, ref_globs, changed_path_globs,
             branch_update_policy, branch_create, branch_delete, tag_create,
             tag_update, tag_delete, other_create, other_update, other_delete,
             request_bytes, pack_bytes, object_count, ref_updates,
             exact_parent_required, expected_parent, normalized_hash)
         SELECT $1, $2, binding.binding_id, generic.resource_id,
                binding.grammar_version, binding.git_operations,
                binding.ref_globs, binding.changed_path_globs,
                binding.branch_update_policy, binding.branch_create,
                binding.branch_delete, binding.tag_create, binding.tag_update,
                binding.tag_delete, binding.other_create, binding.other_update,
                binding.other_delete, binding.request_bytes, binding.pack_bytes,
                binding.object_count, binding.ref_updates,
                binding.exact_parent_required,
                CASE WHEN binding.exact_parent_required
                     THEN provenance.target_commit ELSE NULL END,
                binding.normalized_hash
         FROM agent_instance_revisions AS revision
         JOIN agent_git_capability_bindings AS binding
           ON binding.binding_id = revision.publication_repository_binding_id
          AND binding.instance_revision_id = revision.id
         JOIN agent_capability_bindings AS generic
           ON generic.id = binding.binding_id
          AND generic.instance_revision_id = binding.instance_revision_id
         LEFT JOIN run_instance_provenance AS provenance ON provenance.run_id = $3
         WHERE revision.id = $2",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(run_id)
    .execute(&mut **transaction)
    .await
    .map_err(storage)?;
    Ok(())
}

#[derive(FromRow)]
struct SessionRow {
    id: Uuid,
    snapshot_id: Uuid,
    identity_hash: Vec<u8>,
    issuance_generation: i64,
    status: String,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    acknowledged_at: Option<OffsetDateTime>,
    revoked_at: Option<OffsetDateTime>,
}

impl TryFrom<SessionRow> for StoredRuntimeSession {
    type Error = RuntimeAuthorityError;

    fn try_from(row: SessionRow) -> Result<Self, Self::Error> {
        let identity_hash: [u8; 32] = row
            .identity_hash
            .try_into()
            .map_err(|_| RuntimeAuthorityError::Persistence)?;
        Ok(Self {
            id: RuntimeSessionId::from_uuid(row.id),
            snapshot_id: capability_domain::AuthorizationSnapshotId::from_uuid(row.snapshot_id),
            identity_hash: capability_domain::AuthorityHash::from_bytes(identity_hash),
            generation: generation_from_i64(row.issuance_generation)?,
            status: parse_status(&row.status)?,
            issued_at: row.issued_at,
            expires_at: row.expires_at,
            acknowledged_at: row.acknowledged_at,
            revoked_at: row.revoked_at,
        })
    }
}

fn generation_from_i64(value: i64) -> Result<RuntimeCredentialGeneration, RuntimeAuthorityError> {
    u64::try_from(value)
        .map_err(storage)
        .and_then(|value| RuntimeCredentialGeneration::new(value).map_err(storage))
}

fn parse_status(value: &str) -> Result<RuntimeSessionStatus, RuntimeAuthorityError> {
    match value {
        "pending_handoff" => Ok(RuntimeSessionStatus::PendingHandoff),
        "active" => Ok(RuntimeSessionStatus::Active),
        "revoked" => Ok(RuntimeSessionStatus::Revoked),
        "expired" => Ok(RuntimeSessionStatus::Expired),
        _ => Err(RuntimeAuthorityError::Persistence),
    }
}

fn storage<T>(_: T) -> RuntimeAuthorityError {
    RuntimeAuthorityError::Persistence
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_mailbox_snapshot_binding_is_exact_publish_authority() {
        let binding_id = Uuid::new_v4();
        let mailbox_id = Uuid::new_v4();
        let binding = gateway_mailbox_binding(GatewaySnapshotBindingRow {
            binding_id,
            grant_id: Uuid::new_v4(),
            slot_key: "accepted-event".to_owned(),
            resource_id: mailbox_id,
        })
        .expect("valid exact mailbox publication binding");
        assert_eq!(binding.id().as_uuid(), binding_id);
        assert_eq!(binding.resource().kind, CapabilityResourceKind::Mailbox);
        assert_eq!(binding.resource().id, mailbox_id);
        assert_eq!(
            binding.granted_operations().collect::<Vec<_>>(),
            vec![CapabilityOperation::Publish]
        );
    }

    #[test]
    fn parses_mailbox_publish_persisted_authority() {
        assert_eq!(
            resource_kind("mailbox").expect("mailbox is a supported persisted resource"),
            CapabilityResourceKind::Mailbox
        );
        assert_eq!(
            operations(&["publish".to_owned()]).expect("publish is supported"),
            vec![CapabilityOperation::Publish]
        );
    }
}
