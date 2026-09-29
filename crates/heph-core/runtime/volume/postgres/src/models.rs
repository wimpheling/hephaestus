use runtime_types::{AgentInstanceId, LeaseId, RunId, VolumeId};
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{Volume, VolumeError, VolumeKind, VolumeLease, VolumeState};

#[derive(Debug, FromRow)]
/// Database row for a persisted volume.
pub struct VolumeRow {
    /// Volume identifier.
    pub id: Uuid,
    /// Owning agent instance identifier.
    pub instance_id: Uuid,
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
            instance_id: AgentInstanceId::from_uuid(row.instance_id),
            kind: VolumeKind::InstanceState,
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
            key_reference: row.key_reference,
            encryption_version: row.encryption_version,
            backup_revision: row.backup_revision,
            checksum: row.checksum,
            last_successful_backup_at: row.last_successful_backup_at,
        })
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
