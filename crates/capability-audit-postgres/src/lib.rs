//! `PostgreSQL` persistence and authorized inspection for capability evidence.

use async_trait::async_trait;
use authz_postgres::begin_actor_transaction;
use capability_audit::{
    CapabilityAuditError, CapabilityAuditEventKind, CapabilityAuditPage, CapabilityAuditReason,
    CapabilityAuditRecord, CapabilityAuditRepository, CapabilityDecision, CapabilityUseOutcome,
    NewCapabilityAuditEvent,
};
use capability_domain::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityOperation, CapabilityResource,
    CapabilityResourceKind, CapabilitySlotKey, RuntimeSessionId,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

/// `PostgreSQL` capability audit adapter.
#[derive(Clone)]
pub struct PostgresCapabilityAuditRepository {
    pool: PgPool,
}

impl PostgresCapabilityAuditRepository {
    /// Creates an adapter over shared application/worker pools.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CapabilityAuditRepository for PostgresCapabilityAuditRepository {
    async fn append(&self, event: &NewCapabilityAuditEvent) -> Result<(), CapabilityAuditError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(CapabilityAuditError::provider)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(CapabilityAuditError::provider)?;
        append_in_transaction(&mut transaction, event).await?;
        transaction
            .commit()
            .await
            .map_err(CapabilityAuditError::provider)?;
        Ok(())
    }

    async fn list_for_run(
        &self,
        identity: &AuthenticatedIdentity,
        run_id: RunId,
        page: CapabilityAuditPage,
    ) -> Result<Vec<CapabilityAuditRecord>, CapabilityAuditError> {
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(CapabilityAuditError::provider)?;
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *transaction)
            .await
            .map_err(CapabilityAuditError::provider)?;
        let allowed: bool = sqlx::query_scalar(
            "SELECT check_permission(
                'user', hephaestus_actor_id(), 'can_read', 'run', $1::text
             ) = 1",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(CapabilityAuditError::provider)?;
        if !allowed {
            return Err(CapabilityAuditError::Unavailable);
        }
        let before = page.before();
        let rows = sqlx::query_as::<_, AuditRow>(
            "SELECT id, run_id, instance_id, instance_revision_id,
                    runtime_session_id, snapshot_id, binding_id, slot_key,
                    resource_kind, resource_id, operation, request_id,
                    grantor_id, event_kind, decision, outcome, reason_code,
                    authorization_model_version, occurred_at
             FROM capability_audit_inspection
             WHERE run_id = $1
               AND ($2::timestamptz IS NULL
                    OR (occurred_at, id) < ($2, $3))
             ORDER BY occurred_at DESC, id DESC
             LIMIT $4",
        )
        .bind(run_id.as_uuid())
        .bind(before.map(|cursor| cursor.occurred_at))
        .bind(before.map(|cursor| cursor.id))
        .bind(i64::from(page.limit()))
        .fetch_all(&mut *transaction)
        .await
        .map_err(CapabilityAuditError::provider)?;
        transaction
            .commit()
            .await
            .map_err(CapabilityAuditError::provider)?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
}

#[derive(Debug, FromRow)]
struct AuditRow {
    id: Uuid,
    run_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    runtime_session_id: Uuid,
    snapshot_id: Uuid,
    binding_id: Uuid,
    slot_key: String,
    resource_kind: String,
    resource_id: Uuid,
    grantor_id: Uuid,
    operation: String,
    request_id: Uuid,
    event_kind: String,
    decision: Option<String>,
    outcome: Option<String>,
    reason_code: Option<String>,
    authorization_model_version: String,
    occurred_at: OffsetDateTime,
}

impl TryFrom<AuditRow> for CapabilityAuditRecord {
    type Error = CapabilityAuditError;

    fn try_from(row: AuditRow) -> Result<Self, Self::Error> {
        let kind = match row.event_kind.as_str() {
            "authorization_decision" => CapabilityAuditEventKind::AuthorizationDecision,
            "capability_use" => CapabilityAuditEventKind::CapabilityUse,
            _ => return Err(CapabilityAuditError::InvalidEvidence),
        };
        let decision = match row.decision.as_deref() {
            Some("allow") => Some(CapabilityDecision::Allow),
            Some("deny") => Some(CapabilityDecision::Deny),
            None => None,
            Some(_) => return Err(CapabilityAuditError::InvalidEvidence),
        };
        let outcome = match row.outcome.as_deref() {
            Some("succeeded") => Some(CapabilityUseOutcome::Succeeded),
            Some("failed") => Some(CapabilityUseOutcome::Failed),
            None => None,
            Some(_) => return Err(CapabilityAuditError::InvalidEvidence),
        };
        if (kind == CapabilityAuditEventKind::AuthorizationDecision
            && (decision.is_none() || outcome.is_some()))
            || (kind == CapabilityAuditEventKind::CapabilityUse
                && (decision.is_some() || outcome.is_none()))
        {
            return Err(CapabilityAuditError::InvalidEvidence);
        }
        let resource_kind = resource_kind(&row.resource_kind)?;
        Ok(Self {
            id: row.id,
            run_id: RunId::from_uuid(row.run_id),
            instance_id: AgentInstanceId::from_uuid(row.instance_id),
            instance_revision_id: AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
            runtime_session_id: RuntimeSessionId::from_uuid(row.runtime_session_id),
            snapshot_id: AuthorizationSnapshotId::from_uuid(row.snapshot_id),
            binding_id: CapabilityBindingId::from_uuid(row.binding_id),
            slot: CapabilitySlotKey::parse(row.slot_key)
                .map_err(|_| CapabilityAuditError::InvalidEvidence)?,
            resource: CapabilityResource::new(resource_kind, row.resource_id),
            grantor_id: UserId::from_uuid(row.grantor_id),
            operation: operation(&row.operation)?,
            request_id: RequestId::from_uuid(row.request_id),
            kind,
            decision,
            outcome,
            reason: row
                .reason_code
                .map(CapabilityAuditReason::parse)
                .transpose()?,
            authorization_model_version: row.authorization_model_version,
            occurred_at: row.occurred_at,
        })
    }
}

/// Appends capability evidence in an existing trusted worker transaction.
///
/// A capability adapter whose controlled mutation is stored in `PostgreSQL`
/// should use this function so the use outcome and mutation commit atomically.
/// The caller must already have selected the trusted worker role.
///
/// # Errors
///
/// Fails closed when the event does not match the exact immutable snapshot or
/// when durable storage rejects it.
pub async fn append_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    event: &NewCapabilityAuditEvent,
) -> Result<(), CapabilityAuditError> {
    sqlx::query(
        "INSERT INTO capability_audit_events
            (id, runtime_session_id, snapshot_id, binding_id, request_id,
             event_kind, operation, decision, outcome, reason_code,
             authorization_model_version, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(event.id)
    .bind(event.context.runtime_session_id.as_uuid())
    .bind(event.context.snapshot_id.as_uuid())
    .bind(event.context.binding_id.as_uuid())
    .bind(event.context.request_id.as_uuid())
    .bind(event.kind.as_str())
    .bind(event.context.operation.as_str())
    .bind(event.decision.map(CapabilityDecision::as_str))
    .bind(event.outcome.map(CapabilityUseOutcome::as_str))
    .bind(event.reason.as_ref().map(CapabilityAuditReason::as_str))
    .bind(event.context.authorization_model_version)
    .bind(event.occurred_at)
    .execute(&mut **transaction)
    .await
    .map_err(CapabilityAuditError::provider)?;
    Ok(())
}

fn resource_kind(value: &str) -> Result<CapabilityResourceKind, CapabilityAuditError> {
    match value {
        "repository" => Ok(CapabilityResourceKind::Repository),
        "project" => Ok(CapabilityResourceKind::Project),
        "agent_instance" => Ok(CapabilityResourceKind::AgentInstance),
        "gateway" => Ok(CapabilityResourceKind::Gateway),
        "run" => Ok(CapabilityResourceKind::Run),
        "state_volume" => Ok(CapabilityResourceKind::StateVolume),
        _ => Err(CapabilityAuditError::InvalidEvidence),
    }
}

fn operation(value: &str) -> Result<CapabilityOperation, CapabilityAuditError> {
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
        _ => Err(CapabilityAuditError::InvalidEvidence),
    }
}
