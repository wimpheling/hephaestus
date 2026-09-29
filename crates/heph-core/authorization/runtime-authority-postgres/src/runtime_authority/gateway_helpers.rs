use super::common::storage;
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, CapabilityBinding, CapabilityBindingId,
    CapabilityOperation, CapabilityRequirement, CapabilityRequirementId, CapabilityResource,
    CapabilityResourceKind, CapabilitySlotKey, WorkloadKind, WorkloadPrincipal,
};
use runtime_authority::{GatewayRuntimeSessionRequest, RuntimeAuthorityError};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn ensure_gateway_handler_contract(
    pool: &PgPool,
    request: GatewayRuntimeSessionRequest,
    expected: &str,
) -> Result<(), RuntimeAuthorityError> {
    let actual = sqlx::query_scalar::<_, String>(
        "SELECT handler_contract FROM gateway_revisions
         WHERE id = $1 AND gateway_id = $2",
    )
    .bind(request.gateway_revision_id)
    .bind(request.gateway_id)
    .fetch_optional(pool)
    .await
    .map_err(storage)?;
    if actual.as_deref() == Some(expected) {
        Ok(())
    } else {
        Err(RuntimeAuthorityError::IdentityMismatch)
    }
}

pub(super) async fn gateway_snapshot(
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

pub(super) async fn insert_gateway_snapshot_bindings(
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
pub(super) struct GatewaySnapshotBindingRow {
    pub(super) binding_id: Uuid,
    pub(super) grant_id: Uuid,
    pub(super) slot_key: String,
    pub(super) resource_id: Uuid,
}

pub(super) fn gateway_mailbox_binding(
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

pub(super) fn stored_binding(
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
pub(super) struct SnapshotBindingRow {
    pub(super) binding_id: Uuid,
    pub(super) requirement_id: Uuid,
    pub(super) slot_key: String,
    pub(super) resource_kind: String,
    pub(super) required_operations: Vec<String>,
    pub(super) optional_operations: Vec<String>,
    pub(super) slot_required: bool,
    pub(super) requirement_hash: Vec<u8>,
    pub(super) resource_id: Uuid,
    pub(super) granted_operations: Vec<String>,
    pub(super) binding_hash: Vec<u8>,
    pub(super) authorization_model_version: String,
}

pub(super) fn resource_kind(value: &str) -> Result<CapabilityResourceKind, RuntimeAuthorityError> {
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

pub(super) fn operations(
    values: &[String],
) -> Result<Vec<CapabilityOperation>, RuntimeAuthorityError> {
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
