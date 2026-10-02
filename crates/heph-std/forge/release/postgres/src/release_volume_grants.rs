//! Audited actor operations for explicit immutable volume mount authority.

use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use identity_domain::{AuthenticatedIdentity, UserId};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_domain::{VolumeMountGrantId, VolumeMountRevocationId, VolumeMountScope};
use volume_trait::{
    VolumeError, VolumeMountGrant, VolumeMountGrantRepository, VolumeMountRevocation,
};

use crate::release_volume_grant_rows::{GrantRow, storage};
use crate::{ReleaseService, ReleaseServiceError};

#[async_trait]
impl VolumeMountGrantRepository for ReleaseService {
    async fn grant_volume_mount(
        &self,
        identity: &AuthenticatedIdentity,
        grant_id: VolumeMountGrantId,
        scope: &VolumeMountScope,
    ) -> Result<VolumeMountGrant, VolumeError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let grant = grant_in_transaction(self, &mut tx, identity, grant_id, scope).await?;
        tx.commit().await.map_err(storage)?;
        Ok(grant)
    }

    async fn revoke_volume_mount(
        &self,
        identity: &AuthenticatedIdentity,
        revocation_id: VolumeMountRevocationId,
        grant_id: VolumeMountGrantId,
    ) -> Result<VolumeMountRevocation, VolumeError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        let grant = load_grant(&mut tx, grant_id)
            .await?
            .ok_or(VolumeError::PermissionDenied)?;
        // V1 requires a same-project binding. The frozen0102 source CanManage
        // and consumer CanManage are the same owning-project maintainer gate.
        // Checking consumer management therefore also accepts source owners,
        // without recording a spurious denial for an allowed alternative.
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(
                ObjectType::AgentInstance,
                grant.scope.instance_id().as_uuid(),
            ),
        )
        .await
        .map_err(authorization_error)?;
        sqlx::query(
            "INSERT INTO agent_instance_volume_mount_revocations
             (id,grant_id,created_by,request_id,authorization_model_version)
             VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
        )
        .bind(revocation_id.as_uuid())
        .bind(grant_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        let row: Option<(Uuid,Uuid,Uuid)> = sqlx::query_as(
            "SELECT id,grant_id,created_by FROM agent_instance_volume_mount_revocations WHERE id=$1",
        ).bind(revocation_id.as_uuid()).fetch_optional(&mut *tx).await.map_err(storage)?;
        let (id, stored_grant, creator) = row.ok_or(VolumeError::IntentConflict)?;
        if stored_grant != grant_id.as_uuid() || creator != identity.user_id.as_uuid() {
            return Err(VolumeError::IntentConflict);
        }
        tx.commit().await.map_err(storage)?;
        Ok(VolumeMountRevocation {
            id: VolumeMountRevocationId::from_uuid(id).map_err(storage)?,
            grant_id,
            created_by: UserId::from_uuid(creator),
        })
    }
}

async fn load_grant(
    tx: &mut Transaction<'_, Postgres>,
    id: VolumeMountGrantId,
) -> Result<Option<VolumeMountGrant>, VolumeError> {
    sqlx::query_as::<_, GrantRow>(
        "SELECT id,instance_revision_id,instance_id,release_agent_id,slot_key,volume_id,
         access_mode,release_contract_hash,created_by,authorization_model_version
         FROM agent_instance_volume_mount_grants WHERE id=$1",
    )
    .bind(id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .map(TryInto::try_into)
    .transpose()
}

fn authorization_error(error: ReleaseServiceError) -> VolumeError {
    match error {
        ReleaseServiceError::AuthorizationDenied => VolumeError::PermissionDenied,
        other => storage(other),
    }
}

/// Creates a grant inside an existing actor transaction, for future atomic
/// initial import. The consumer revision may be inactive or not runnable;
/// runtime dispatch independently requires both activation and runnable state.
pub async fn grant_in_transaction(
    service: &ReleaseService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    grant_id: VolumeMountGrantId,
    scope: &VolumeMountScope,
) -> Result<VolumeMountGrant, VolumeError> {
    service
        .require(
            tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, scope.instance_id().as_uuid()),
        )
        .await
        .map_err(authorization_error)?;
    for permission in [Permission::CanGrantAgentCapability, Permission::CanAttach] {
        service
            .require(
                tx,
                identity,
                permission,
                ObjectRef::new(ObjectType::StateVolume, scope.volume_id().as_uuid()),
            )
            .await
            .map_err(authorization_error)?;
    }
    // Unique(revision,slot) permanently prevents replacement after revoke.
    // The database validates the exact declaration and rejects update candidates.
    sqlx::query(
        "INSERT INTO agent_instance_volume_mount_grants
                (id,instance_revision_id,instance_id,release_agent_id,slot_key,
                 volume_id,access_mode,release_contract_hash,created_by,request_id,
                 authorization_model_version)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT DO NOTHING",
    )
    .bind(grant_id.as_uuid())
    .bind(scope.revision_id().as_uuid())
    .bind(scope.instance_id().as_uuid())
    .bind(scope.release_agent_id().as_uuid())
    .bind(scope.slot().as_str())
    .bind(scope.volume_id().as_uuid())
    .bind(scope.access_mode().as_str())
    .bind(scope.release_contract_hash().as_bytes().as_slice())
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .bind(AUTHORIZATION_MODEL_VERSION)
    .execute(&mut **tx)
    .await
    .map_err(storage)?;
    let grant = load_grant(tx, grant_id)
        .await?
        .ok_or(VolumeError::IntentConflict)?;
    if grant.scope != *scope || grant.created_by != identity.user_id {
        return Err(VolumeError::IntentConflict);
    }
    let revoked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=$1)",
    )
    .bind(grant_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    if revoked {
        return Err(VolumeError::InvalidState(
            "mount grant is permanently revoked",
        ));
    }
    Ok(grant)
}
