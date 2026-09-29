//! Single-host raw-file volumes with injected durable metadata.

use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use time::OffsetDateTime;
use tokio::{fs, process::Command};
use volume_trait::{
    INSTANCE_STATE_DISK_ID, Volume, VolumeAttachment, VolumeError, VolumeLease,
    VolumeMetadataRepository, VolumeStore,
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

/// Filesystem-backed volume store over provider-neutral metadata.
#[derive(Clone)]
pub struct LocalVolumeStore {
    metadata: Arc<dyn VolumeMetadataRepository>,
    config: LocalVolumeConfig,
}

impl LocalVolumeStore {
    /// Creates a store after validating its static configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the backing roots, host identity, or lease
    /// duration is invalid.
    pub fn new(
        metadata: Arc<dyn VolumeMetadataRepository>,
        config: LocalVolumeConfig,
    ) -> Result<Self, VolumeError> {
        if !config.volume_root.is_absolute() {
            return Err(invalid_backing("volume_root must be absolute"));
        }
        if config.host_id.is_empty() {
            return Err(invalid_backing("host_id must not be empty"));
        }
        if config.transient_runtime_roots.iter().any(|runtime_root| {
            config.volume_root.starts_with(runtime_root)
                || runtime_root.starts_with(&config.volume_root)
        }) {
            return Err(invalid_backing(
                "persistent volume_root overlaps a transient VM runtime root",
            ));
        }
        if config.lease_duration.is_zero() {
            return Err(invalid_backing("lease_duration must be greater than zero"));
        }
        Ok(Self { metadata, config })
    }

    /// Creates the configured backing root. Database migrations belong to the
    /// injected metadata adapter.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be created or canonicalized.
    pub async fn initialize(&self) -> Result<(), VolumeError> {
        fs::create_dir_all(&self.config.volume_root)
            .await
            .map_err(backing)?;
        let canonical = fs::canonicalize(&self.config.volume_root)
            .await
            .map_err(backing)?;
        if canonical != self.config.volume_root {
            return Err(invalid_backing("volume_root must be canonical"));
        }
        Ok(())
    }

    async fn initialize_backing(&self, volume: &Volume) -> Result<(), VolumeError> {
        let path = &volume.host_path;
        ensure_direct_child(&self.config.volume_root, path)?;
        match fs::symlink_metadata(path).await {
            Ok(metadata) => {
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                    return Err(invalid_backing("volume backing path is not a regular file"));
                }
                let file = fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .await
                    .map_err(backing)?;
                file.set_len(volume.capacity_bytes).await.map_err(backing)?;
                file.sync_all().await.map_err(backing)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let file = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(path)
                    .await
                    .map_err(backing)?;
                file.set_len(volume.capacity_bytes).await.map_err(backing)?;
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
            return Err(invalid_backing(format!("mkfs.ext4 exited with {status}")));
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

    fn expiry(&self, now: OffsetDateTime) -> Result<OffsetDateTime, VolumeError> {
        let duration = time::Duration::try_from(self.config.lease_duration).map_err(backing)?;
        now.checked_add(duration)
            .ok_or_else(|| invalid_backing("lease expiry overflow"))
    }
}

#[async_trait]
impl VolumeStore for LocalVolumeStore {
    async fn resolve_instance_state(
        &self,
        instance_id: AgentInstanceId,
        capacity_bytes: u64,
    ) -> Result<Volume, VolumeError> {
        if capacity_bytes < MINIMUM_CAPACITY_BYTES {
            return Err(invalid_backing(
                "instance-state capacity must be at least 16 MiB",
            ));
        }
        // The metadata adapter owns the durable volume ID; it derives the
        // final `<volume-id>.raw` name from this configured-root hint.
        let path = self.config.volume_root.join("volume.raw");
        let volume = self
            .metadata
            .resolve_instance_state(
                instance_id,
                capacity_bytes,
                &self.config.host_id,
                &path,
                uuid::Uuid::new_v4(),
            )
            .await?;
        if matches!(volume.state, volume_trait::VolumeState::Uninitialized) {
            self.initialize_backing(&volume).await?;
            self.metadata.mark_ready(volume.id).await?;
        }
        self.metadata.volume(volume.id).await
    }

    async fn acquire(
        &self,
        volume_id: VolumeId,
        run_id: RunId,
    ) -> Result<VolumeAttachment, VolumeError> {
        let now = OffsetDateTime::now_utc();
        let lease = self
            .metadata
            .acquire(
                volume_id,
                run_id,
                &self.config.host_id,
                now,
                self.expiry(now)?,
            )
            .await?;
        Ok(VolumeAttachment {
            volume: self.metadata.volume(volume_id).await?,
            lease,
            disk_id: INSTANCE_STATE_DISK_ID,
        })
    }

    async fn mark_attached(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        let now = OffsetDateTime::now_utc();
        self.metadata
            .mark_attached(lease, now, self.expiry(now)?)
            .await
    }

    async fn heartbeat(&self, lease: &VolumeLease) -> Result<VolumeLease, VolumeError> {
        let now = OffsetDateTime::now_utc();
        self.metadata.heartbeat(lease, now, self.expiry(now)?).await
    }

    async fn active_lease_for_run(
        &self,
        run_id: RunId,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        self.metadata
            .active_lease_for_run(run_id, &self.config.host_id)
            .await
    }

    async fn release_after_detach(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        self.metadata.release_after_detach(lease, false).await
    }

    async fn stale_leases(&self, now: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        self.metadata.stale_leases(now).await
    }

    async fn begin_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        self.metadata.begin_recovery(lease).await
    }

    async fn finish_recovery(&self, lease: &VolumeLease) -> Result<(), VolumeError> {
        self.metadata.release_after_detach(lease, true).await
    }
}

fn ensure_direct_child(root: &Path, path: &Path) -> Result<(), VolumeError> {
    if path.parent() != Some(root)
        || path.extension().and_then(|value| value.to_str()) != Some("raw")
    {
        return Err(invalid_backing(
            "volume backing path escapes its configured root",
        ));
    }
    Ok(())
}

fn invalid_backing(message: impl Into<String>) -> VolumeError {
    VolumeError::Backing(Box::new(io::Error::new(
        io::ErrorKind::InvalidInput,
        message.into(),
    )))
}

fn backing(error: impl std::error::Error + Send + Sync + 'static) -> VolumeError {
    VolumeError::Backing(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::ensure_direct_child;
    use std::path::Path;

    #[test]
    fn backing_path_must_be_a_direct_raw_child() {
        let root = Path::new("/var/lib/hephaestus/volumes");
        assert!(ensure_direct_child(root, &root.join("volume.raw")).is_ok());
        assert!(ensure_direct_child(root, &root.join("../escape.raw")).is_err());
        assert!(ensure_direct_child(root, &root.join("volume.img")).is_err());
    }
}
