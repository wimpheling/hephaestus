//! Permanent launch closure admission; physical drain is a later worker boundary.

use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use release_domain::InstanceRemovalId;
use release_service::{InstanceRemovalAdmission, RequestInstanceRemoval};
use uuid::Uuid;

use crate::release_volume_instance_inputs::exact_replay;
use crate::{ReleaseService, ReleaseServiceError};

impl ReleaseService {
    /// Permanently closes future launches and admits supervised cancellation.
    ///
    /// Current consumer managers can withdraw after source or creator authority
    /// is lost. No volume is deleted and no terminal cleanup receipt is created.
    ///
    /// # Errors
    ///
    /// Rejects current management denial before replay, stale versions, conflicting
    /// closure identities, changed input, unavailable instances, or storage failure.
    pub async fn request_instance_removal(
        &self,
        identity: &AuthenticatedIdentity,
        command: RequestInstanceRemoval,
    ) -> Result<InstanceRemovalAdmission, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        if command.instance_id.as_uuid().is_nil() || command.removal_id.as_uuid().is_nil() {
            return Err(ReleaseServiceError::InvalidUpdateLifecycle);
        }
        let hash = crate::release_instance_removal_work::input_hash(identity, &command)?;
        if let Some((instance, Some(removal))) = exact_replay(
            &mut tx,
            command.command_key,
            "request_instance_removal",
            &hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(InstanceRemovalAdmission {
                instance_id: release_domain::AgentInstanceId::from_uuid(instance),
                removal_id: InstanceRemovalId::from_uuid(removal),
            });
        }
        let observed: Option<(i64, String)> =
            sqlx::query_as("SELECT version,state FROM agent_instances WHERE id=$1 FOR UPDATE")
                .bind(command.instance_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?;
        let (version, state) = observed.ok_or(ReleaseServiceError::Unavailable)?;
        if u64::try_from(version).ok() != Some(command.expected_version) || state == "removed" {
            return Err(ReleaseServiceError::StaleInstanceRevision);
        }
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM agent_instance_removal_requests WHERE instance_id=$1",
        )
        .bind(command.instance_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?;
        if existing.is_some() {
            return Err(ReleaseServiceError::IdempotencyConflict);
        }
        crate::release_instance_removal_work::admit_request(&mut tx, identity, &command, &hash)
            .await?;
        tx.commit().await?;
        Ok(InstanceRemovalAdmission {
            instance_id: command.instance_id,
            removal_id: command.removal_id,
        })
    }
}
