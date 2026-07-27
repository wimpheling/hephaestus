//! Single-host raw-file volumes with `PostgreSQL` ownership and lease metadata.

use async_trait::async_trait;
use runtime_types::{AgentId, LeaseId, RunId, VolumeId};
use sqlx::{FromRow, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use time::OffsetDateTime;
use tokio::{fs, process::Command};
use uuid::Uuid;
use volume_trait::{
    AGENT_STATE_DISK_ID, Volume, VolumeAttachment, VolumeError, VolumeKind, VolumeLease,
    VolumeState, VolumeStore,
};

const MINIMUM_CAPACITY_BYTES: u64 = 16 * 1024 * 1024;

/// Configuration for [`LocalVolumeStore`].
#[derive(Debug, Clone)]
pub struct LocalVolumeConfig {
    /// Persistent root containing caller-owned raw backing files.
    pub volume_root: PathBuf,
    /// Transient VM roots that must not overlap persistent storage.
    pub transient_runtime_roots: Vec<PathBuf>,
    /// Stable identity of this single host.
    pub host_id: String,
    /// Writable lease duration between supervisor heartbeats.
    pub lease_duration: Duration,
    /// Host `mkfs.ext4` executable.
    pub mkfs_ext4: PathBuf,
}

/// `PostgreSQL`-coordinated volume store backed by local raw files.
#[derive(Clone)]
pub struct LocalVolumeStore {
    pool: PgPool,
    config: LocalVolumeConfig,
}

impl LocalVolumeStore {
    /// Creates a store after validating its static configuration.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid root, host identity, or lease duration.
    pub fn new(pool: PgPool, config: LocalVolumeConfig) -> Result<Self, VolumeError> {
        if !config.volume_root.is_absolute() {
            return Err(VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "volume_root must be absolute",
            ))));
        }
        if config.host_id.is_empty() {
            return Err(VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "host_id must not be empty",
            ))));
        }
        if config.transient_runtime_roots.iter().any(|runtime_root| {
            config.volume_root.starts_with(runtime_root)
                || runtime_root.starts_with(&config.volume_root)
        }) {
            return Err(VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "persistent volume_root overlaps a transient VM runtime root",
            ))));
        }
        if config.lease_duration.is_zero() {
            return Err(VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lease_duration must be greater than zero",
            ))));
        }
        Ok(Self { pool, config })
    }

    /// Applies the ordered runtime database migrations and creates the volume
    /// root.
    ///
    /// # Errors
    ///
    /// Returns an error when `PostgreSQL` or local storage initialization fails.
    pub async fn initialize(&self) -> Result<(), VolumeError> {
        sqlx::migrate!("../../migrations")
            .run(&self.pool)
            .await
            .map_err(metadata)?;
        fs::create_dir_all(&self.config.volume_root)
            .await
            .map_err(backing)?;
        let canonical = fs::canonicalize(&self.config.volume_root)
            .await
            .map_err(backing)?;
        if canonical != self.config.volume_root {
            return Err(VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "volume_root must be canonical",
            ))));
        }
        Ok(())
    }

    async fn initialize_backing(&self, volume: &VolumeRow) -> Result<(), VolumeError> {
        let capacity = u64::try_from(volume.capacity_bytes).map_err(backing)?;
        let path = Path::new(&volume.host_path);
        ensure_direct_child(&self.config.volume_root, path)?;

        match fs::symlink_metadata(path).await {
            Ok(metadata) => {
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err(VolumeError::Backing(Box::new(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "volume backing path is not a regular file",
                    ))));
                }
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .await
                    .map_err(backing)?;
                file.set_len(capacity).await.map_err(backing)?;
                file.sync_all().await.map_err(backing)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(path)
                    .await
                    .map_err(backing)?;
                file.set_len(capacity).await.map_err(backing)?;
                file.sync_all().await.map_err(backing)?;
            }
            Err(error) => return Err(backing(error)),
        }

        let status = Command::new(&self.config.mkfs_ext4)
            .arg("-q")
            .arg("-F")
            .arg("-U")
            .arg(volume.filesystem_uuid.to_string())
            .arg(path)
            .status()
            .await
            .map_err(backing)?;
        if !status.success() {
            return Err(VolumeError::Backing(Box::new(io::Error::other(format!(
                "mkfs.ext4 exited with {status}"
            )))));
        }
        fs::OpenOptions::new()
            .read(true)
            .open(path)
            .await
            .map_err(backing)?
            .sync_all()
            .await
            .map_err(backing)
    }

    async fn locked_volume(
        transaction: &mut Transaction<'_, Postgres>,
        volume_id: VolumeId,
    ) -> Result<VolumeRow, VolumeError> {
        sqlx::query_as::<_, VolumeRow>("SELECT * FROM agent_state_volumes WHERE id = $1 FOR UPDATE")
            .bind(volume_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(metadata)?
            .ok_or(VolumeError::NotFound(volume_id))
    }

    fn expiry(&self, now: OffsetDateTime) -> Result<OffsetDateTime, VolumeError> {
        let duration = time::Duration::try_from(self.config.lease_duration).map_err(backing)?;
        now.checked_add(duration).ok_or_else(|| {
            VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lease expiry overflow",
            )))
        })
    }
}

#[async_trait]
impl VolumeStore for LocalVolumeStore {
    async fn resolve_agent_state(
        &self,
        agent_id: AgentId,
        capacity_bytes: u64,
    ) -> Result<Volume, VolumeError> {
        if capacity_bytes < MINIMUM_CAPACITY_BYTES {
            return Err(VolumeError::Backing(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "agent-state capacity must be at least 16 MiB",
            ))));
        }
        let capacity = i64::try_from(capacity_bytes).map_err(backing)?;
        let volume_id = VolumeId::new();
        let filesystem_uuid = Uuid::new_v4();
        let host_path = self
            .config
            .volume_root
            .join(format!("{volume_id}.raw"))
            .into_os_string()
            .into_string()
            .map_err(|_| {
                VolumeError::Backing(Box::new(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "volume path is not UTF-8",
                )))
            })?;

        let mut transaction = self.pool.begin().await.map_err(metadata)?;
        sqlx::query("INSERT INTO agents (id) VALUES ($1) ON CONFLICT (id) DO NOTHING")
            .bind(agent_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(metadata)?;
        sqlx::query(
            "INSERT INTO agent_state_volumes
             (id, agent_id, kind, host_id, host_path, capacity_bytes, filesystem_uuid, state)
             VALUES ($1, $2, 'agent_state', $3, $4, $5, $6, 'uninitialized')
             ON CONFLICT (agent_id, kind) DO NOTHING",
        )
        .bind(volume_id.as_uuid())
        .bind(agent_id.as_uuid())
        .bind(&self.config.host_id)
        .bind(host_path)
        .bind(capacity)
        .bind(filesystem_uuid)
        .execute(&mut *transaction)
        .await
        .map_err(metadata)?;
        transaction.commit().await.map_err(metadata)?;

        let mut transaction = self.pool.begin().await.map_err(metadata)?;
        let row = sqlx::query_as::<_, VolumeRow>(
            "SELECT * FROM agent_state_volumes
             WHERE agent_id = $1 AND kind = 'agent_state'
             FOR UPDATE",
        )
        .bind(agent_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(metadata)?;
        assert_host(&row.host_id, &self.config.host_id)?;
        if row.state == "uninitialized" {
            self.initialize_backing(&row).await?;
            sqlx::query(
                "UPDATE agent_state_volumes
                 SET state = 'ready', updated_at = now()
                 WHERE id = $1 AND state = 'uninitialized'",
            )
            .bind(row.id)
            .execute(&mut *transaction)
            .await
            .map_err(metadata)?;
        }
        transaction.commit().await.map_err(metadata)?;
        self.volume(VolumeId::from_uuid(row.id)).await
    }

    async fn acquire(
        &self,
        volume_id: VolumeId,
        run_id: RunId,
    ) -> Result<VolumeAttachment, VolumeError> {
        let mut transaction = self.pool.begin().await.map_err(metadata)?;
        let volume = Self::locked_volume(&mut transaction, volume_id).await?;
        assert_host(&volume.host_id, &self.config.host_id)?;
        if let Some(existing) = active_lease(&mut transaction, volume_id).await? {
            if existing.run_id == run_id.as_uuid() {
                transaction.commit().await.map_err(metadata)?;
                return Ok(VolumeAttachment {
                    volume: self.volume(volume_id).await?,
                    lease: existing.try_into()?,
                    disk_id: AGENT_STATE_DISK_ID,
                });
            }
            return Err(VolumeError::LeaseConflict {
                volume_id,
                holder_run_id: RunId::from_uuid(existing.run_id),
            });
        }
        if volume.state != "ready" {
            return Err(VolumeError::InvalidState(
                "only a ready volume can be leased",
            ));
        }

        let now = OffsetDateTime::now_utc();
        let generation = volume
            .lease_generation
            .checked_add(1)
            .ok_or(VolumeError::InvalidState("lease generation overflow"))?;
        let lease_id = LeaseId::new();
        sqlx::query(
            "INSERT INTO volume_leases
             (id, volume_id, run_id, host_id, fencing_token,
              acquired_at, heartbeat_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $6, $7)",
        )
        .bind(lease_id.as_uuid())
        .bind(volume_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(&self.config.host_id)
        .bind(generation)
        .bind(now)
        .bind(self.expiry(now)?)
        .execute(&mut *transaction)
        .await
        .map_err(metadata)?;
        sqlx::query(
            "UPDATE agent_state_volumes
             SET lease_generation = $2, updated_at = now()
             WHERE id = $1",
        )
        .bind(volume_id.as_uuid())
        .bind(generation)
        .execute(&mut *transaction)
        .await
        .map_err(metadata)?;
        transaction.commit().await.map_err(metadata)?;

        Ok(VolumeAttachment {
            volume: self.volume(volume_id).await?,
            lease: self.lease(lease_id).await?,
            disk_id: AGENT_STATE_DISK_ID,
        })
    }

    async fn mark_attached(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        let now = OffsetDateTime::now_utc();
        let result = sqlx::query(
            "WITH current_lease AS (
                 UPDATE volume_leases
                 SET attached_at = COALESCE(attached_at, $4),
                     heartbeat_at = $4,
                     expires_at = $5
                 WHERE id = $1 AND volume_id = $2 AND fencing_token = $3
                   AND released_at IS NULL
                 RETURNING volume_id
             )
             UPDATE agent_state_volumes
             SET state = 'attached', updated_at = $4
             WHERE id IN (SELECT volume_id FROM current_lease)
               AND state IN ('ready', 'attached')",
        )
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.fencing_token)
        .bind(now)
        .bind(self.expiry(now)?)
        .execute(&self.pool)
        .await
        .map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        self.lease(lease.id).await
    }

    async fn heartbeat(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        let now = OffsetDateTime::now_utc();
        let result = sqlx::query(
            "UPDATE volume_leases SET heartbeat_at = $4, expires_at = $5
             WHERE id = $1 AND volume_id = $2 AND fencing_token = $3
               AND released_at IS NULL AND recovering_at IS NULL",
        )
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.fencing_token)
        .bind(now)
        .bind(self.expiry(now)?)
        .execute(&self.pool)
        .await
        .map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        self.lease(lease.id).await
    }

    async fn active_lease_for_run(
        &self,
        run_id: RunId,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        let row = sqlx::query_as::<_, LeaseRow>(
            "SELECT * FROM volume_leases
             WHERE run_id = $1 AND released_at IS NULL",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(metadata)?;
        if let Some(row) = row {
            assert_host(&row.host_id, &self.config.host_id)?;
            Ok(Some(row.try_into()?))
        } else {
            Ok(None)
        }
    }

    async fn release_after_detach(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        release(&self.pool, lease, false).await
    }

    async fn stale_leases(&self, now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        sqlx::query_as::<_, LeaseRow>(
            "SELECT * FROM volume_leases
             WHERE released_at IS NULL AND expires_at <= $1
             ORDER BY expires_at, id",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(metadata)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn begin_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        let result = sqlx::query(
            "WITH current_lease AS (
                 UPDATE volume_leases SET recovering_at = COALESCE(recovering_at, now())
                 WHERE id = $1 AND volume_id = $2 AND fencing_token = $3
                   AND released_at IS NULL AND expires_at <= now()
                 RETURNING volume_id
             )
             UPDATE agent_state_volumes SET state = 'recovering', updated_at = now()
             WHERE id IN (SELECT volume_id FROM current_lease)",
        )
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.fencing_token)
        .execute(&self.pool)
        .await
        .map_err(metadata)?;
        if result.rows_affected() == 0 {
            return Err(VolumeError::StaleLease);
        }
        Ok(())
    }

    async fn finish_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        release(&self.pool, lease, true).await
    }
}

impl LocalVolumeStore {
    async fn volume(&self, id: VolumeId) -> Result<Volume, VolumeError> {
        sqlx::query_as::<_, VolumeRow>("SELECT * FROM agent_state_volumes WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(metadata)?
            .ok_or(VolumeError::NotFound(id))?
            .try_into()
    }

    async fn lease(&self, id: LeaseId) -> Result<VolumeLease, VolumeError> {
        sqlx::query_as::<_, LeaseRow>("SELECT * FROM volume_leases WHERE id = $1")
            .bind(id.as_uuid())
            .fetch_one(&self.pool)
            .await
            .map_err(metadata)?
            .try_into()
    }
}

async fn active_lease(
    transaction: &mut Transaction<'_, Postgres>,
    volume_id: VolumeId,
) -> Result<Option<LeaseRow>, VolumeError> {
    sqlx::query_as::<_, LeaseRow>(
        "SELECT * FROM volume_leases
         WHERE volume_id = $1 AND released_at IS NULL
         FOR UPDATE",
    )
    .bind(volume_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(metadata)
}

async fn release(pool: &PgPool, lease: &VolumeLease, recovering: bool) -> Result<(), VolumeError> {
    let mut transaction = pool.begin().await.map_err(metadata)?;
    let condition = if recovering {
        "recovering_at IS NOT NULL"
    } else {
        "recovering_at IS NULL"
    };
    let query = format!(
        "UPDATE volume_leases SET released_at = now()
         WHERE id = $1 AND volume_id = $2 AND fencing_token = $3
           AND released_at IS NULL AND {condition}"
    );
    let result = sqlx::query(&query)
        .bind(lease.id.as_uuid())
        .bind(lease.volume_id.as_uuid())
        .bind(lease.fencing_token)
        .execute(&mut *transaction)
        .await
        .map_err(metadata)?;
    if result.rows_affected() == 0 {
        return Err(VolumeError::StaleLease);
    }
    sqlx::query(
        "UPDATE agent_state_volumes SET state = 'ready', updated_at = now()
         WHERE id = $1 AND lease_generation = $2",
    )
    .bind(lease.volume_id.as_uuid())
    .bind(lease.fencing_token)
    .execute(&mut *transaction)
    .await
    .map_err(metadata)?;
    transaction.commit().await.map_err(metadata)
}

#[derive(Debug)]
struct VolumeRow {
    id: Uuid,
    agent_id: Uuid,
    kind: String,
    host_id: String,
    host_path: String,
    capacity_bytes: i64,
    filesystem_uuid: Uuid,
    state: String,
    lease_generation: i64,
    key_reference: Option<String>,
    encryption_version: Option<i32>,
    backup_revision: Option<i64>,
    checksum: Option<String>,
    last_successful_backup_at: Option<OffsetDateTime>,
}

impl<'row> FromRow<'row, PgRow> for VolumeRow {
    fn from_row(row: &'row PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            kind: row.try_get("kind")?,
            host_id: row.try_get("host_id")?,
            host_path: row.try_get("host_path")?,
            capacity_bytes: row.try_get("capacity_bytes")?,
            filesystem_uuid: row.try_get("filesystem_uuid")?,
            state: row.try_get("state")?,
            lease_generation: row.try_get("lease_generation")?,
            key_reference: row.try_get("key_reference")?,
            encryption_version: row.try_get("encryption_version")?,
            backup_revision: row.try_get("backup_revision")?,
            checksum: row.try_get("checksum")?,
            last_successful_backup_at: row.try_get("last_successful_backup_at")?,
        })
    }
}

impl TryFrom<VolumeRow> for Volume {
    type Error = VolumeError;

    fn try_from(row: VolumeRow) -> Result<Self, Self::Error> {
        let kind = match row.kind.as_str() {
            "agent_state" => VolumeKind::AgentState,
            _ => return Err(VolumeError::InvalidState("unknown volume kind")),
        };
        let state = parse_state(&row.state)?;
        Ok(Self {
            id: VolumeId::from_uuid(row.id),
            agent_id: AgentId::from_uuid(row.agent_id),
            kind,
            host_id: row.host_id,
            host_path: PathBuf::from(row.host_path),
            capacity_bytes: u64::try_from(row.capacity_bytes).map_err(backing)?,
            filesystem_uuid: row.filesystem_uuid,
            state,
            key_reference: row.key_reference,
            encryption_version: row.encryption_version,
            backup_revision: row.backup_revision,
            checksum: row.checksum,
            last_successful_backup_at: row.last_successful_backup_at,
        })
    }
}

#[derive(Debug)]
struct LeaseRow {
    id: Uuid,
    volume_id: Uuid,
    run_id: Uuid,
    host_id: String,
    fencing_token: i64,
    acquired_at: OffsetDateTime,
    heartbeat_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    attached_at: Option<OffsetDateTime>,
}

impl<'row> FromRow<'row, PgRow> for LeaseRow {
    fn from_row(row: &'row PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            volume_id: row.try_get("volume_id")?,
            run_id: row.try_get("run_id")?,
            host_id: row.try_get("host_id")?,
            fencing_token: row.try_get("fencing_token")?,
            acquired_at: row.try_get("acquired_at")?,
            heartbeat_at: row.try_get("heartbeat_at")?,
            expires_at: row.try_get("expires_at")?,
            attached_at: row.try_get("attached_at")?,
        })
    }
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

fn ensure_direct_child(root: &Path, path: &Path) -> Result<(), VolumeError> {
    if path.parent() != Some(root)
        || path.extension().and_then(|value| value.to_str()) != Some("raw")
    {
        return Err(VolumeError::Backing(Box::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            "volume backing path escapes its configured root",
        ))));
    }
    Ok(())
}

fn assert_host(owner_host: &str, requested_host: &str) -> Result<(), VolumeError> {
    if owner_host == requested_host {
        Ok(())
    } else {
        Err(VolumeError::WrongHost {
            owner_host: owner_host.to_owned(),
            requested_host: requested_host.to_owned(),
        })
    }
}

fn metadata(error: impl std::error::Error + Send + Sync + 'static) -> VolumeError {
    VolumeError::Metadata(Box::new(error))
}

fn backing(error: impl std::error::Error + Send + Sync + 'static) -> VolumeError {
    VolumeError::Backing(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::{LocalVolumeConfig, LocalVolumeStore, ensure_direct_child};
    use sqlx::postgres::PgPoolOptions;
    use std::{path::Path, time::Duration};
    use tempfile::TempDir;

    #[test]
    fn backing_path_must_be_a_direct_raw_child() {
        let root = Path::new("/var/lib/hephaestus/volumes");
        assert!(ensure_direct_child(root, &root.join("volume.raw")).is_ok());
        assert!(ensure_direct_child(root, &root.join("../escape.raw")).is_err());
        assert!(ensure_direct_child(root, &root.join("volume.img")).is_err());
    }

    #[tokio::test]
    async fn configuration_rejects_relative_roots() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/hephaestus")
            .unwrap();
        let temp = TempDir::new().unwrap();
        let result = LocalVolumeStore::new(
            pool,
            LocalVolumeConfig {
                volume_root: Path::new("relative").to_path_buf(),
                transient_runtime_roots: Vec::new(),
                host_id: String::from("host"),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: temp.path().join("mkfs.ext4"),
            },
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn configuration_rejects_transient_runtime_overlap() {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/hephaestus")
            .unwrap();
        let temp = TempDir::new().unwrap();
        let result = LocalVolumeStore::new(
            pool,
            LocalVolumeConfig {
                volume_root: temp.path().join("runtime/volumes"),
                transient_runtime_roots: vec![temp.path().join("runtime")],
                host_id: String::from("host"),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: temp.path().join("mkfs.ext4"),
            },
        );
        assert!(result.is_err());
    }
}
