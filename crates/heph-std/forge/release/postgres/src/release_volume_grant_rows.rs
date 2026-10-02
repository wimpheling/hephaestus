//! Validated reconstruction of immutable typed mount authority records.

use capability_domain::{AuthorityHash, CapabilitySlotKey};
use identity_domain::UserId;
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, VolumeId};
use uuid::Uuid;
use volume_domain::{VolumeAccessMode, VolumeMountGrantId, VolumeMountScope};
use volume_trait::{VolumeError, VolumeMountGrant};

#[derive(sqlx::FromRow)]
pub struct GrantRow {
    pub id: Uuid,
    pub instance_revision_id: Uuid,
    pub instance_id: Uuid,
    pub release_agent_id: Uuid,
    pub slot_key: String,
    pub volume_id: Uuid,
    pub access_mode: String,
    pub release_contract_hash: Vec<u8>,
    pub created_by: Uuid,
    pub authorization_model_version: String,
}

impl TryFrom<GrantRow> for VolumeMountGrant {
    type Error = VolumeError;
    fn try_from(row: GrantRow) -> Result<Self, Self::Error> {
        let access_mode = match row.access_mode.as_str() {
            "read_only" => VolumeAccessMode::ReadOnly,
            "read_write" => VolumeAccessMode::ReadWrite,
            _ => return Err(VolumeError::InvalidState("invalid mount grant mode")),
        };
        let hash = row
            .release_contract_hash
            .try_into()
            .map_err(|_| VolumeError::InvalidState("invalid mount grant hash"))?;
        Ok(Self {
            id: VolumeMountGrantId::from_uuid(row.id).map_err(storage)?,
            scope: VolumeMountScope::new(
                AgentInstanceId::from_uuid(row.instance_id),
                AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
                ReleaseAgentId::from_uuid(row.release_agent_id),
                CapabilitySlotKey::parse(row.slot_key).map_err(storage)?,
                VolumeId::from_uuid(row.volume_id),
                access_mode,
                AuthorityHash::from_bytes(hash),
            )
            .map_err(storage)?,
            created_by: UserId::from_uuid(row.created_by),
            authorization_model_version: row.authorization_model_version,
        })
    }
}

pub fn storage(error: impl std::error::Error + Send + Sync + 'static) -> VolumeError {
    VolumeError::Metadata(Box::new(error))
}
