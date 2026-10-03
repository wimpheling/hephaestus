//! Actor resource operations: live policy checks and durable decision audit.

use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use runtime_types::VolumeId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{VolumeError, VolumeInspection, VolumeRegistration, VolumeResourceRepository};

use crate::{PostgresVolumeMetadataRepository, errors::metadata, models::VolumeRow};

async fn authorized(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    permission: Permission,
    object: ObjectRef,
) -> Result<bool, VolumeError> {
    let decision = PostgresMelangeAuthorizer
        .check(tx, Subject::User(identity.user_id), permission, object)
        .await
        .map_err(metadata)?;
    audit_decision(
        tx,
        identity.user_id,
        permission,
        object,
        decision,
        identity.request_id,
    )
    .await
    .map_err(metadata)?;
    Ok(decision.is_allowed())
}

#[async_trait]
impl VolumeResourceRepository for PostgresVolumeMetadataRepository {
    async fn register(
        &self,
        identity: &AuthenticatedIdentity,
        intent: &VolumeRegistration,
    ) -> Result<VolumeInspection, VolumeError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(metadata)?;
        if !authorized(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, intent.project_id()),
        )
        .await?
        {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::PermissionDenied);
        }
        sqlx::query("INSERT INTO agent_instance_state_volumes (id, project_id, state, capacity_bytes, filesystem_uuid) VALUES ($1,$2,'uninitialized',$3,$4) ON CONFLICT (id) DO NOTHING")
            .bind(intent.id().as_uuid()).bind(intent.project_id())
            .bind(i64::try_from(intent.capacity_bytes()).map_err(metadata)?)
            .bind(intent.filesystem_uuid()).execute(&mut *tx).await.map_err(metadata)?;
        let row = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE",
        )
        .bind(intent.id().as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(metadata)?;
        let Some(row) = row else {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::PermissionDenied);
        };
        if row.project_id != intent.project_id()
            || row.instance_id.is_some()
            || row.capacity_bytes != i64::try_from(intent.capacity_bytes()).map_err(metadata)?
            || row.filesystem_uuid != Some(intent.filesystem_uuid())
        {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::IntentConflict);
        }
        let result = row.try_into()?;
        tx.commit().await.map_err(metadata)?;
        Ok(result)
    }

    async fn inspect(
        &self,
        identity: &AuthenticatedIdentity,
        volume_id: VolumeId,
    ) -> Result<VolumeInspection, VolumeError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(metadata)?;
        if !authorized(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::StateVolume, volume_id.as_uuid()),
        )
        .await?
        {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::PermissionDenied);
        }
        let row = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_instance_state_volumes WHERE id=$1",
        )
        .bind(volume_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(metadata)?;
        let result = row.ok_or(VolumeError::PermissionDenied)?.try_into()?;
        tx.commit().await.map_err(metadata)?;
        Ok(result)
    }

    async fn list(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        after: Option<VolumeId>,
        limit: u32,
    ) -> Result<Vec<VolumeInspection>, VolumeError> {
        if !(1..=100).contains(&limit) {
            return Err(VolumeError::InvalidState(
                "volume list limit must be between 1 and 100",
            ));
        }
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(metadata)?;
        if !authorized(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Project, project_id),
        )
        .await?
        {
            tx.commit().await.map_err(metadata)?;
            return Err(VolumeError::PermissionDenied);
        }
        // Apply exact resource checks in SQL before limiting: readers cannot
        // infer hidden resources from empty pages or cursor movement.
        let rows = sqlx::query_as::<_, VolumeRow>("SELECT * FROM agent_instance_state_volumes WHERE project_id=$1 AND ($2::uuid IS NULL OR id>$2) AND check_permission('user',$3,'can_read','state_volume',id::text)=1 ORDER BY id LIMIT $4")
            .bind(project_id).bind(after.map(VolumeId::as_uuid))
            .bind(identity.user_id.to_string()).bind(i64::from(limit))
            .fetch_all(&mut *tx).await.map_err(metadata)?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            // Exact per-resource decisions are recorded in the same transaction.
            if authorized(
                &mut tx,
                identity,
                Permission::CanRead,
                ObjectRef::new(ObjectType::StateVolume, row.id),
            )
            .await?
            {
                result.push(row.try_into()?);
            }
        }
        tx.commit().await.map_err(metadata)?;
        Ok(result)
    }
}
