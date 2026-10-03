use runtime_types::{AgentInstanceId, LeaseId, RunId, VolumeId};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{
    Volume, VolumeError, VolumeInspection, VolumeKind, VolumeLease, VolumeProvisioningState,
    VolumeState,
};

#[derive(Debug, FromRow)]
/// Database row for a persisted volume.
pub struct VolumeRow {
    /// Volume identifier.
    pub id: Uuid,
    /// Owning agent instance identifier.
    pub instance_id: Option<Uuid>,
    /// Stable project that owns this resource.
    pub project_id: Uuid,
    /// Durable provisioning progress.
    pub provisioning_state: String,
    /// Independent provisioning fence.
    pub provisioning_generation: i64,
    /// Owning host identifier.
    pub host_id: Option<String>,
    /// Host filesystem path.
    pub host_path: Option<String>,
    /// Allocated capacity in bytes.
    pub capacity_bytes: i64,
    /// Filesystem UUID.
    pub filesystem_uuid: Option<Uuid>,
    /// Persisted lifecycle state.
    pub state: String,
    /// Monotonic lease fencing generation.
    pub lease_generation: i64,
    /// Optional encryption key reference.
    pub key_reference: Option<String>,
    /// Optional encryption version.
    pub encryption_version: Option<i32>,
    /// Optional backup revision.
    pub backup_revision: Option<i64>,
    /// Optional backup checksum.
    pub checksum: Option<String>,
    /// Last successful backup timestamp.
    pub last_successful_backup_at: Option<OffsetDateTime>,
}

impl TryFrom<VolumeRow> for Volume {
    type Error = VolumeError;

    fn try_from(row: VolumeRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: VolumeId::from_uuid(row.id),
            project_id: row.project_id,
            instance_id: row.instance_id.map(AgentInstanceId::from_uuid),
            kind: if row.instance_id.is_some() {
                VolumeKind::InstanceState
            } else {
                VolumeKind::Private
            },
            host_id: row
                .host_id
                .ok_or(VolumeError::InvalidState("volume has no host owner"))?,
            host_path: row
                .host_path
                .ok_or(VolumeError::InvalidState("volume has no host path"))?
                .into(),
            capacity_bytes: u64::try_from(row.capacity_bytes).map_err(super::errors::metadata)?,
            filesystem_uuid: row
                .filesystem_uuid
                .ok_or(VolumeError::InvalidState("volume has no filesystem UUID"))?,
            state: parse_state(&row.state)?,
            provisioning_state: parse_provisioning_state(&row.provisioning_state)?,
            provisioning_generation: row.provisioning_generation,
            key_reference: row.key_reference,
            encryption_version: row.encryption_version,
            backup_revision: row.backup_revision,
            checksum: row.checksum,
            last_successful_backup_at: row.last_successful_backup_at,
        })
    }
}

impl TryFrom<VolumeRow> for VolumeInspection {
    type Error = VolumeError;

    fn try_from(row: VolumeRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: VolumeId::from_uuid(row.id),
            project_id: row.project_id,
            legacy_origin_instance_id: row.instance_id.map(AgentInstanceId::from_uuid),
            capacity_bytes: u64::try_from(row.capacity_bytes).map_err(super::errors::metadata)?,
            filesystem_uuid: row.filesystem_uuid,
            provisioning_state: parse_provisioning_state(&row.provisioning_state)?,
            lifecycle_state: parse_state(&row.state)?,
        })
    }
}

fn parse_provisioning_state(value: &str) -> Result<VolumeProvisioningState, VolumeError> {
    match value {
        "reserved" => Ok(VolumeProvisioningState::Reserved),
        "creating" => Ok(VolumeProvisioningState::Creating),
        "formatting" => Ok(VolumeProvisioningState::Formatting),
        "ready" => Ok(VolumeProvisioningState::Ready),
        "uncertain" => Ok(VolumeProvisioningState::Uncertain),
        _ => Err(VolumeError::InvalidState("unknown provisioning state")),
    }
}

#[derive(Debug, FromRow)]
/// Database row for a volume lease.
pub struct LeaseRow {
    /// Lease identifier.
    pub id: Uuid,
    /// Volume identifier.
    pub volume_id: Uuid,
    /// Run holding the lease.
    pub run_id: Uuid,
    /// Host holding the lease.
    pub host_id: String,
    /// Optimistic fencing token.
    pub fencing_token: i64,
    /// Lease acquisition timestamp.
    pub acquired_at: OffsetDateTime,
    /// Last heartbeat timestamp.
    pub heartbeat_at: OffsetDateTime,
    /// Lease expiry timestamp.
    pub expires_at: OffsetDateTime,
    /// Optional attach timestamp.
    pub attached_at: Option<OffsetDateTime>,
}

impl TryFrom<LeaseRow> for VolumeLease {
    type Error = VolumeError;

    fn try_from(row: LeaseRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: LeaseId::from_uuid(row.id),
            volume_id: VolumeId::from_uuid(row.volume_id),
            run_id: RunId::from_uuid(row.run_id),
            host_id: row.host_id,
            fencing_token: row.fencing_token,
            acquired_at: row.acquired_at,
            heartbeat_at: row.heartbeat_at,
            expires_at: row.expires_at,
            attached_at: row.attached_at,
        })
    }
}

fn parse_state(value: &str) -> Result<VolumeState, VolumeError> {
    match value {
        "uninitialized" => Ok(VolumeState::Uninitialized),
        "ready" => Ok(VolumeState::Ready),
        "attached" => Ok(VolumeState::Attached),
        "recovering" => Ok(VolumeState::Recovering),
        _ => Err(VolumeError::InvalidState("unknown volume state")),
    }
}
