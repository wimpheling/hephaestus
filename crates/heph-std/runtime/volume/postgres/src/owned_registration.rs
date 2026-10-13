//! Canonical metadata birth seals. No filesystem ownership is inferred here.

use crate::{PostgresVolumeMetadataRepository, errors::metadata};
use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use runtime_types::VolumeId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedVolumeRegistration, OwnedVolumeRegistrationReceipt, VolumeCreationIdentity,
    VolumeCreationIdentityWire, VolumeCreationLineage, VolumeCreationOperationId,
    VolumeCreationSealId, VolumeError, VolumeOwnedRegistrationRepository, VolumeOwnershipScopeId,
    VolumeRegistration,
};

#[derive(sqlx::FromRow)]
struct SealRow {
    id: Uuid,
    volume_id: Uuid,
    project_id: Uuid,
    filesystem_uuid: Uuid,
    capacity_bytes: i64,
    scope_id: Uuid,
    operation_id: Uuid,
    command_id: Uuid,
    effect_id: Uuid,
    attempt_id: Uuid,
    actor_id: Uuid,
    origin_request_id: Uuid,
    resource_version: i64,
    generation: i64,
    input_hash: Vec<u8>,
    intent_hash: Vec<u8>,
    canonical_bytes: Vec<u8>,
    registration_hash: Vec<u8>,
    first_request_id: Uuid,
    event_id: Uuid,
    event_cursor: i64,
    event_aggregate_version: i64,
}

fn hash(bytes: Vec<u8>) -> Result<[u8; 32], VolumeError> {
    bytes
        .try_into()
        .map_err(|_| VolumeError::InvalidState("invalid persisted creation fingerprint"))
}
impl SealRow {
    fn receipt(self) -> Result<OwnedVolumeRegistrationReceipt, VolumeError> {
        let creation = VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
            scope_id: VolumeOwnershipScopeId::from_uuid(self.scope_id).map_err(metadata)?,
            operation_id: VolumeCreationOperationId::from_uuid(self.operation_id)
                .map_err(metadata)?,
            command_id: self.command_id,
            effect_id: self.effect_id,
            attempt_id: self.attempt_id,
            actor_id: self.actor_id,
            request_id: self.origin_request_id,
            resource_version: u64::try_from(self.resource_version).map_err(metadata)?,
            generation: u64::try_from(self.generation).map_err(metadata)?,
            input_hash: hash(self.input_hash)?,
            intent_hash: hash(self.intent_hash)?,
        })
        .map_err(metadata)?;
        let registration = VolumeRegistration::new(
            VolumeId::from_uuid(self.volume_id),
            self.project_id,
            u64::try_from(self.capacity_bytes).map_err(metadata)?,
            self.filesystem_uuid,
        )?;
        let intent = OwnedVolumeRegistration::new(registration, creation);
        if intent.canonical_bytes() != self.canonical_bytes {
            return Err(VolumeError::InvalidState(
                "persisted creation encoding differs from original intent",
            ));
        }
        Ok(OwnedVolumeRegistrationReceipt {
            intent,
            seal_id: VolumeCreationSealId::from_uuid(self.id).map_err(metadata)?,
            registration_hash: hash(self.registration_hash)?,
            first_request_id: self.first_request_id,
            event_id: self.event_id,
            event_cursor: self.event_cursor,
            event_aggregate_version: self.event_aggregate_version,
        })
    }
}

async fn authority(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    object: ObjectRef,
) -> Result<bool, VolumeError> {
    let decision = PostgresMelangeAuthorizer
        .check(
            tx,
            Subject::User(identity.user_id),
            Permission::CanManage,
            object,
        )
        .await
        .map_err(metadata)?;
    audit_decision(
        tx,
        identity.user_id,
        Permission::CanManage,
        object,
        decision,
        identity.request_id,
    )
    .await
    .map_err(metadata)?;
    Ok(decision.is_allowed())
}
async fn role(tx: &mut Transaction<'_, Postgres>, expected: &str) -> Result<bool, VolumeError> {
    let actual: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(metadata)?;
    Ok(actual == expected)
}
pub async fn lineage(
    tx: &mut Transaction<'_, Postgres>,
    id: VolumeId,
) -> Result<VolumeCreationLineage, VolumeError> {
    let row =
        sqlx::query_as::<_, SealRow>("SELECT * FROM volume_creation_seals WHERE volume_id=$1")
            .bind(id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(metadata)?;
    if let Some(row) = row {
        return Ok(VolumeCreationLineage::Owned(Box::new(row.receipt()?)));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=$1)")
            .bind(id.as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(metadata)?;
    Ok(if exists {
        VolumeCreationLineage::Independent
    } else {
        VolumeCreationLineage::Missing
    })
}
async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    intent: &OwnedVolumeRegistration,
) -> Result<Option<OwnedVolumeRegistrationReceipt>, VolumeError> {
    let row = sqlx::query_as::<_, SealRow>(
        "SELECT * FROM volume_creation_seals WHERE operation_id=$1 OR volume_id=$2",
    )
    .bind(intent.creation().fields().operation_id.as_uuid())
    .bind(intent.registration().id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(metadata)?;
    if let Some(row) = row {
        let receipt = row.receipt()?;
        if receipt.intent != *intent {
            return Err(VolumeError::IntentConflict);
        }
        return Ok(Some(receipt));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=$1)")
            .bind(intent.registration().id().as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(metadata)?;
    if exists {
        return Err(VolumeError::IntentConflict);
    }
    Ok(None)
}

fn write_error(error: sqlx::Error) -> VolumeError {
    if error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .as_deref()
        == Some("23505")
    {
        VolumeError::IntentConflict
    } else {
        metadata(error)
    }
}

async fn create(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    intent: &OwnedVolumeRegistration,
) -> Result<OwnedVolumeRegistrationReceipt, VolumeError> {
    let seal = Uuid::new_v4();
    let resource = intent.registration();
    let origin = intent.creation().fields();
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(origin.operation_id.as_uuid().to_string())
        .execute(&mut **tx)
        .await
        .map_err(write_error)?;
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes,filesystem_uuid,creation_seal_id,creation_operation_id) VALUES($1,$2,'uninitialized',$3,$4,$5,$6)")
        .bind(resource.id().as_uuid()).bind(resource.project_id())
        .bind(i64::try_from(resource.capacity_bytes()).map_err(metadata)?).bind(resource.filesystem_uuid())
        .bind(seal).bind(origin.operation_id.as_uuid()).execute(&mut **tx).await.map_err(write_error)?;
    let row=sqlx::query_as::<_, SealRow>("INSERT INTO volume_creation_seals(id,volume_id,project_id,filesystem_uuid,capacity_bytes,scope_id,operation_id,command_id,effect_id,attempt_id,actor_id,origin_request_id,resource_version,generation,input_hash,intent_hash,canonical_bytes,registration_hash,first_request_id,event_id,event_cursor,event_aggregate_version) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,sha256($17),$18,event.id,event.cursor,event.aggregate_version FROM application_events event WHERE event.occurrence_id=$7 AND event.scope_kind='project' AND event.scope_id=$3 AND event.aggregate_type='project' AND event.aggregate_id=$3 AND event.change_kind='created' RETURNING volume_creation_seals.*")
        .bind(seal).bind(resource.id().as_uuid()).bind(resource.project_id()).bind(resource.filesystem_uuid())
        .bind(i64::try_from(resource.capacity_bytes()).map_err(metadata)?)
        .bind(origin.scope_id.as_uuid()).bind(origin.operation_id.as_uuid()).bind(origin.command_id)
        .bind(origin.effect_id).bind(origin.attempt_id).bind(origin.actor_id).bind(origin.request_id)
        .bind(i64::try_from(origin.resource_version).map_err(metadata)?).bind(i64::try_from(origin.generation).map_err(metadata)?)
        .bind(origin.input_hash.as_slice()).bind(origin.intent_hash.as_slice()).bind(intent.canonical_bytes())
        .bind(identity.request_id.as_uuid()).fetch_one(&mut **tx).await.map_err(write_error)?;
    row.receipt()
}

#[async_trait]
impl VolumeOwnedRegistrationRepository for PostgresVolumeMetadataRepository {
    async fn register_owned(
        &self,
        identity: &AuthenticatedIdentity,
        intent: &OwnedVolumeRegistration,
    ) -> Result<OwnedVolumeRegistrationReceipt, VolumeError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(metadata)?;
        // Live authority precedes lookup/replay; historical actor labels grant nothing.
        let allowed = authority(
            &mut tx,
            identity,
            ObjectRef::new(ObjectType::Project, intent.registration().project_id()),
        )
        .await?;
        if !allowed
            || !role(&mut tx, "hephaestus_app").await?
            || identity.user_id.as_uuid() != intent.creation().fields().actor_id
        {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::PermissionDenied);
        }
        // Serialize competing operations in this project without a global lock.
        sqlx::query("SELECT id FROM projects WHERE id=$1 FOR UPDATE")
            .bind(intent.registration().project_id())
            .execute(&mut *tx)
            .await
            .map_err(metadata)?;
        let receipt = match replay(&mut tx, intent).await? {
            Some(receipt) => receipt,
            None => create(&mut tx, identity, intent).await?,
        };
        tx.commit().await.map_err(metadata)?;
        Ok(receipt)
    }
    async fn inspect_creation(
        &self,
        identity: &AuthenticatedIdentity,
        volume_id: VolumeId,
    ) -> Result<VolumeCreationLineage, VolumeError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(metadata)?;
        if !authority(
            &mut tx,
            identity,
            ObjectRef::new(ObjectType::StateVolume, volume_id.as_uuid()),
        )
        .await?
            || !role(&mut tx, "hephaestus_app").await?
        {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::PermissionDenied);
        }
        let result = lineage(&mut tx, volume_id).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(result)
    }
    async fn creation_lineage(
        &self,
        volume_id: VolumeId,
    ) -> Result<VolumeCreationLineage, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(metadata)?;
        if !role(&mut tx, "hephaestus_worker").await? {
            return Err(VolumeError::PermissionDenied);
        }
        let result = lineage(&mut tx, volume_id).await?;
        tx.commit().await.map_err(metadata)?;
        Ok(result)
    }
}
