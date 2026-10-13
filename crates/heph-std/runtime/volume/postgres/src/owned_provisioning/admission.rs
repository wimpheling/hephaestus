use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use sqlx::{Postgres, Transaction};
use volume_trait::{BeginOwnedProvisioning, OwnedProvisioningContext, VolumeError};

use super::storage;
use crate::{PostgresVolumeMetadataRepository, errors::metadata};

pub async fn begin(
    repository: &PostgresVolumeMetadataRepository,
    identity: &AuthenticatedIdentity,
    request: &BeginOwnedProvisioning,
) -> Result<OwnedProvisioningContext, VolumeError> {
    let purpose = &request.purpose;
    let resource = purpose.receipt().intent.registration();
    let mut tx = begin_actor_transaction(&repository.pool, identity)
        .await
        .map_err(metadata)?;
    storage::role(&mut tx, "hephaestus_app").await?;
    let decision = PostgresMelangeAuthorizer
        .check(
            &mut tx,
            Subject::User(identity.user_id),
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, resource.project_id()),
        )
        .await
        .map_err(metadata)?;
    audit_decision(
        &mut tx,
        identity.user_id,
        Permission::CanManage,
        ObjectRef::new(ObjectType::Project, resource.project_id()),
        decision,
        identity.request_id,
    )
    .await
    .map_err(metadata)?;
    if !decision.is_allowed() {
        tx.commit().await.map_err(metadata)?;
        return Err(VolumeError::PermissionDenied);
    }
    if identity.user_id.as_uuid() != purpose.receipt().intent.creation().fields().actor_id {
        tx.commit().await.map_err(metadata)?;
        return Err(VolumeError::PermissionDenied);
    }
    if request.operation_id.is_nil() || request.expected_generation >= i64::MAX.cast_unsigned() {
        return Err(VolumeError::IntentConflict);
    }
    let (generation, state): (i64, String) = sqlx::query_as("SELECT provisioning_generation,state FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(resource.id().as_uuid()).fetch_optional(&mut *tx).await.map_err(metadata)?
        .ok_or(VolumeError::IntentConflict)?;
    storage::original_receipt(&mut tx, purpose).await?;
    storage::require_live(&mut tx, resource.id().as_uuid()).await?;
    let known = storage::check_purpose(&mut tx, purpose).await?;
    if let Some(row) = storage::operation(&mut tx, request.operation_id).await? {
        if !known
            || row.expected_generation
                != i64::try_from(request.expected_generation).map_err(metadata)?
        {
            return Err(VolumeError::IntentConflict);
        }
        let result = storage::context(&mut tx, row.claim(purpose)?).await?;
        tx.commit().await.map_err(metadata)?;
        return Ok(result);
    }
    if u64::try_from(generation).map_err(metadata)? != request.expected_generation {
        return Err(VolumeError::StaleLease);
    }
    if state != "uninitialized" {
        return Err(VolumeError::InvalidState(
            "owned backing cannot be reprovisioned",
        ));
    }
    if !known {
        insert_purpose(&mut tx, request).await?;
    }
    let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM volume_owned_provisioning_operations operation WHERE operation.volume_id=$1 AND NOT EXISTS(SELECT 1 FROM volume_owned_backing_observations observation WHERE observation.operation_id=operation.id AND observation.phase='failed_before_format'))")
        .bind(resource.id().as_uuid()).fetch_one(&mut *tx).await.map_err(metadata)?;
    if active {
        return Err(VolumeError::ProvisioningUncertain(
            "another original operation requires reconciliation",
        ));
    }
    create_operation(&mut tx, identity, request).await?;
    let row = storage::operation(&mut tx, request.operation_id)
        .await?
        .ok_or(VolumeError::IntentConflict)?;
    let result = storage::context(&mut tx, row.claim(purpose)?).await?;
    tx.commit().await.map_err(metadata)?;
    Ok(result)
}
async fn insert_purpose(
    tx: &mut Transaction<'_, Postgres>,
    request: &BeginOwnedProvisioning,
) -> Result<(), VolumeError> {
    let purpose = &request.purpose;
    let receipt = purpose.receipt();
    sqlx::query("INSERT INTO volume_owned_backing_purposes(volume_id,seal_id,project_id,host_id,root_path,owner_namespace,namespace_name,canonical_path,birth_generation,canonical_bytes,purpose_hash) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,sha256($10))")
        .bind(receipt.intent.registration().id().as_uuid()).bind(receipt.seal_id.as_uuid()).bind(receipt.intent.registration().project_id())
        .bind(purpose.host()).bind(purpose.root().to_str().ok_or(VolumeError::IntentConflict)?)
        .bind(purpose.owner_namespace().as_uuid()).bind(purpose.namespace_name())
        .bind(purpose.canonical_path().to_str().ok_or(VolumeError::IntentConflict)?)
        .bind(i64::try_from(purpose.birth_generation()).map_err(metadata)?).bind(purpose.canonical_bytes())
        .execute(&mut **tx).await.map_err(metadata)?;
    Ok(())
}
async fn create_operation(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    request: &BeginOwnedProvisioning,
) -> Result<(), VolumeError> {
    let purpose = &request.purpose;
    let id = purpose.receipt().intent.registration().id().as_uuid();
    let expected = i64::try_from(request.expected_generation).map_err(metadata)?;
    storage::occurrence(tx, request.operation_id).await?;
    sqlx::query("INSERT INTO volume_owned_transition_proofs(id,volume_id,operation_id,generation,previous_generation,mode) VALUES($1,$2,$1,$3,$4,'claim')")
        .bind(request.operation_id).bind(id).bind(expected+1).bind(expected).execute(&mut **tx).await.map_err(metadata)?;
    sqlx::query("UPDATE agent_instance_state_volumes SET host_id=$2,host_path=$3,provisioning_generation=$4,updated_at=now() WHERE id=$1 AND provisioning_generation=$5")
        .bind(id).bind(purpose.host()).bind(purpose.canonical_path().to_str().ok_or(VolumeError::IntentConflict)?)
        .bind(expected+1).bind(expected).execute(&mut **tx).await.map_err(metadata)?;
    sqlx::query("INSERT INTO volume_owned_provisioning_operations(id,volume_id,purpose_hash,expected_generation,generation,actor_id,request_id,event_id) SELECT $1,$2,$3,$4,$4+1,$5,$6,event.id FROM application_events event WHERE event.occurrence_id=$1 AND event.scope_kind='project' AND event.scope_id=$7 AND event.aggregate_type='project' AND event.aggregate_id=$7 AND event.event_type='project.changed'")
        .bind(request.operation_id).bind(id).bind(purpose.hash().as_slice()).bind(expected)
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(purpose.receipt().intent.registration().project_id())
        .execute(&mut **tx).await.map_err(metadata)?;
    Ok(())
}
