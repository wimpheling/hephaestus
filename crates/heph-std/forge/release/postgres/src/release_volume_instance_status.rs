//! Actor-visible profile and permanent closure admission.

use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use release_domain::{AgentInstanceId, InstanceRemovalId, InstanceVolumeMode};
use release_service::InstanceVolumeStatus;
use uuid::Uuid;

use crate::{ReleaseService, ReleaseServiceError};

#[derive(sqlx::FromRow)]
struct StatusRow {
    version: i64,
    volume_mode: String,
    run_gate_open: bool,
    supported: bool,
    removal_id: Option<Uuid>,
}

impl ReleaseService {
    /// Returns the live profile after exact instance read authorization.
    ///
    /// `dispatch_supported=false` diagnoses unsupported named execution without
    /// changing the revision's runnable completeness. A removal ID records
    /// admission, without claiming runs or volumes have been cleaned up.
    ///
    /// # Errors
    ///
    /// Fails for read denial, missing instance, invalid stored mode, or storage error.
    pub async fn instance_volume_status(
        &self,
        identity: &AuthenticatedIdentity,
        instance: AgentInstanceId,
    ) -> Result<InstanceVolumeStatus, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::AgentInstance, instance.as_uuid()),
        )
        .await?;
        let row: StatusRow = sqlx::query_as("SELECT instance.version,instance.volume_mode,instance.run_gate_open,(instance.volume_mode='legacy' OR named_volume_dispatch_supported(instance.id)) AS supported,removal.id AS removal_id FROM agent_instances instance LEFT JOIN agent_instance_removal_requests removal ON removal.instance_id=instance.id WHERE instance.id=$1")
            .bind(instance.as_uuid()).fetch_optional(&mut *tx).await?.ok_or(ReleaseServiceError::Unavailable)?;
        let mode = match row.volume_mode.as_str() {
            "legacy" => InstanceVolumeMode::Legacy,
            "named" => InstanceVolumeMode::Named,
            _ => return Err(ReleaseServiceError::InvalidStoredData),
        };
        let version =
            u64::try_from(row.version).map_err(|_| ReleaseServiceError::InvalidStoredData)?;
        tx.commit().await?;
        Ok(InstanceVolumeStatus {
            instance_id: instance,
            version,
            mode,
            dispatch_supported: row.supported,
            run_gate_open: row.run_gate_open,
            removal_id: row.removal_id.map(InstanceRemovalId::from_uuid),
        })
    }
}
