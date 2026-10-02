//! Permanent launch closure admission; physical drain is a later worker boundary.

use authz_domain::{ObjectRef, ObjectType, Permission};
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use release_domain::InstanceRemovalId;
use release_service::{InstanceRemovalAdmission, RequestInstanceRemoval};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::release_volume_instance_inputs::{exact_replay, record_exact};
use crate::{ReleaseService, ReleaseServiceError, append_instance_event};

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
        let hash:[u8;32]=Sha256::digest(serde_json::to_vec(&json!({"version":1,"operation":"request_instance_removal","actor":identity.user_id,"instance":command.instance_id,"removal":command.removal_id,"expected_version":command.expected_version}))?).into();
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
        sqlx::query("INSERT INTO agent_instance_removal_requests(id,instance_id,expected_version,input_hash,created_by,request_id,authorization_model_version) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(command.removal_id.as_uuid()).bind(command.instance_id.as_uuid()).bind(version).bind(hash.as_slice()).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(AUTHORIZATION_MODEL_VERSION).execute(&mut *tx).await?;
        crate::release_instance_removal_work::admit_cancellation(&mut tx, identity, &command)
            .await?;
        record_exact(
            &mut tx,
            command.command_key,
            "request_instance_removal",
            command.instance_id.as_uuid(),
            Some(command.removal_id.as_uuid()),
            identity,
            &hash,
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            None,
            "instance.removal_requested",
            identity,
            json!({"removal_id":command.removal_id,"cleanup_complete":false}),
        )
        .await?;
        tx.commit().await?;
        Ok(InstanceRemovalAdmission {
            instance_id: command.instance_id,
            removal_id: command.removal_id,
        })
    }
}
