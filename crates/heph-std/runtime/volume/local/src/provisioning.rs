//! Exclusive local creation and proof-based recovery; existing bytes are retained.

use std::{
    fs::File,
    os::{
        fd::AsRawFd,
        unix::fs::{FileExt, MetadataExt},
    },
    path::Path,
};

use runtime_types::VolumeId;
use rustix::fs::{FlockOperation, Mode, OFlags};
use tokio::process::Command;
use volume_trait::{ProvisioningClaim, Volume, VolumeError, VolumeProvisioningState, VolumeState};

use crate::{LocalVolumeStore, backing, ensure_direct_child, invalid_backing};

mod attachment;

impl LocalVolumeStore {
    /// Provisions a registered resource as a trusted host worker.
    ///
    /// No caller-supplied host path is accepted. A persisted reservation and
    /// exclusive file lock precede all backing effects. Existing bytes are only
    /// inspected; ambiguous files are retained and never resized or reformatted.
    ///
    /// # Errors
    ///
    /// Returns an error for conflicting handles, unsupported encryption,
    /// concurrent provisioning, unknown filesystem identity, or incomplete IO.
    pub async fn provision(&self, volume_id: VolumeId) -> Result<Volume, VolumeError> {
        let volume = self
            .metadata
            .reserve_provider(volume_id, &self.config.host_id, &self.config.volume_root)
            .await?;
        validate_intent(&volume, &self.config.volume_root, &self.config.host_id)?;
        let root = open_root(&self.config.volume_root)?;
        let lock = open_lock(&root, volume_id)?;
        // The OS lock releases on process death. A later worker may inspect the
        // previous attempt, but a stale claim never permits another mkfs call.
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| VolumeError::InvalidState("volume provisioning is already locked"))?;
        let current = self.metadata.volume(volume_id).await?;
        validate_intent(&current, &self.config.volume_root, &self.config.host_id)?;
        if current.provisioning_state == VolumeProvisioningState::Ready {
            return Ok(current);
        }
        if current.state != VolumeState::Uninitialized {
            return Err(VolumeError::InvalidState(
                "only an uninitialized volume can be provisioned",
            ));
        }
        let claim = self.metadata.claim_provisioning(volume_id).await?;
        let result = self.provision_locked(&root, &lock, &claim).await;
        if result.is_err() {
            // Failure to persist this uncertainty must itself fail the operation.
            // No later retry will truncate the retained backing either way.
            self.metadata
                .provisioning_progress(&claim, VolumeProvisioningState::Uncertain)
                .await?;
        }
        result?;
        self.metadata.volume(volume_id).await
    }

    async fn provision_locked(
        &self,
        root: &File,
        lock: &File,
        claim: &ProvisioningClaim,
    ) -> Result<(), VolumeError> {
        let name = format!("{}.raw", claim.volume.id);
        match open_existing(root, &name) {
            Ok(file) => {
                validate_file(&file)?;
                self.prove_filesystem(&file, lock, &claim.volume).await?;
            }
            Err(rustix::io::Errno::NOENT) => {
                // Absence is observed under the same host lock and reserved
                // namespace. Exclusive creation is the final absence proof.
                self.metadata
                    .provisioning_progress(claim, VolumeProvisioningState::Creating)
                    .await?;
                let file: File = rustix::fs::openat(
                    root,
                    name.as_str(),
                    OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::RDWR
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                )
                .map_err(backing)?
                .into();
                validate_file(&file)?;
                file.set_len(claim.volume.capacity_bytes).map_err(backing)?;
                file.sync_all().map_err(backing)?;
                root.sync_all().map_err(backing)?;
                self.metadata
                    .provisioning_progress(claim, VolumeProvisioningState::Formatting)
                    .await?;
                // The descriptor remains open in this process: mkfs targets the
                // exact newly-created inode through procfs, never a mutable path.
                let status = locked_command(&self.config.mkfs_ext4, lock)?
                    .args(["-q", "-F", "-U"])
                    .arg(claim.volume.filesystem_uuid.to_string())
                    .arg(descriptor_path(&file))
                    .status()
                    .await
                    .map_err(backing)?;
                if !status.success() {
                    return Err(VolumeError::ProvisioningUncertain(
                        "filesystem format did not complete",
                    ));
                }
                file.sync_all().map_err(backing)?;
                self.prove_filesystem(&file, lock, &claim.volume).await?;
            }
            Err(error) => return Err(backing(error)),
        }
        self.metadata
            .provisioning_progress(claim, VolumeProvisioningState::Ready)
            .await
    }

    async fn prove_filesystem(
        &self,
        file: &File,
        lock: &File,
        volume: &Volume,
    ) -> Result<(), VolumeError> {
        prove_ext4(file, volume.capacity_bytes, volume.filesystem_uuid)?;
        // A superblock alone can survive a interrupted mkfs. A complete readonly
        // consistency check is required before an ambiguous attempt is adopted.
        let executable = self
            .config
            .mkfs_ext4
            .parent()
            .ok_or_else(|| invalid_backing("mkfs.ext4 must have a parent directory"))?
            .join("e2fsck");
        let status = locked_command(&executable, lock)?
            .args(["-f", "-n"])
            .arg(descriptor_path(file))
            .status()
            .await
            .map_err(backing)?;
        if !status.success() {
            return Err(VolumeError::ProvisioningUncertain(
                "filesystem consistency cannot be proven",
            ));
        }
        Ok(())
    }
}

fn locked_command(executable: &Path, lock: &File) -> Result<Command, VolumeError> {
    let mut command = Command::new(executable);
    // The child inherits the same flock through stdin. Cancellation can drop
    // the parent's descriptors while the child is still exiting; exclusion
    // remains until the actual subprocess releases its inherited descriptor.
    command.stdin(std::process::Stdio::from(
        lock.try_clone().map_err(backing)?,
    ));
    command.kill_on_drop(true);
    Ok(command)
}

fn validate_intent(volume: &Volume, root: &Path, host_id: &str) -> Result<(), VolumeError> {
    ensure_direct_child(root, &volume.host_path)?;
    if volume.host_path != root.join(format!("{}.raw", volume.id)) || volume.host_id != host_id {
        return Err(VolumeError::IntentConflict);
    }
    if volume.key_reference.is_some() || volume.encryption_version.is_some() {
        return Err(VolumeError::InvalidState(
            "encrypted volume provisioning is not implemented",
        ));
    }
    if !(volume_trait::MIN_LOCAL_VOLUME_CAPACITY_BYTES
        ..=volume_trait::MAX_REGISTERED_VOLUME_CAPACITY_BYTES)
        .contains(&volume.capacity_bytes)
        || volume.capacity_bytes % 4096 != 0
        || volume.filesystem_uuid.is_nil()
    {
        return Err(VolumeError::InvalidState(
            "reserved volume capacity or filesystem identity is unsupported",
        ));
    }
    Ok(())
}

fn open_root(path: &Path) -> Result<File, VolumeError> {
    let file: File = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(backing)?
    .into();
    let metadata = file.metadata().map_err(backing)?;
    if metadata.mode() & 0o022 != 0 || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(invalid_backing(
            "volume root must be owned by the provider and not writable by others",
        ));
    }
    Ok(file)
}

fn open_lock(root: &File, id: VolumeId) -> Result<File, VolumeError> {
    let name = format!(".{id}.lock");
    let file: File = rustix::fs::openat(
        root,
        name.as_str(),
        OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(backing)?
    .into();
    validate_file(&file)?;
    root.sync_all().map_err(backing)?;
    Ok(file)
}

fn open_existing(root: &File, name: &str) -> Result<File, rustix::io::Errno> {
    rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(Into::into)
}

fn validate_file(file: &File) -> Result<(), VolumeError> {
    let metadata = file.metadata().map_err(backing)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(invalid_backing(
            "volume backing must be a private regular inode without hard links",
        ));
    }
    Ok(())
}

fn descriptor_path(file: &File) -> String {
    format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd())
}

fn prove_ext4(file: &File, capacity: u64, expected_uuid: uuid::Uuid) -> Result<(), VolumeError> {
    if file.metadata().map_err(backing)?.len() != capacity {
        return Err(VolumeError::ProvisioningUncertain(
            "backing capacity differs from its intent",
        ));
    }
    let mut superblock = [0_u8; 1024];
    file.read_exact_at(&mut superblock, 1024).map_err(backing)?;
    if superblock[56..58] != [0x53, 0xef] || superblock[104..120] != *expected_uuid.as_bytes() {
        return Err(VolumeError::ProvisioningUncertain(
            "ext4 filesystem identity differs from its intent",
        ));
    }
    if read_u32(&superblock, 96) & 0x40 == 0 {
        return Err(VolumeError::ProvisioningUncertain(
            "filesystem lacks ext4 extents",
        ));
    }
    let low = u64::from(read_u32(&superblock, 4));
    let high = if read_u32(&superblock, 96) & 0x80 != 0 {
        u64::from(read_u32(&superblock, 336))
    } else {
        0
    };
    let log_block_size = read_u32(&superblock, 24);
    let block_size = 1024_u64
        .checked_shl(log_block_size)
        .filter(|size| *size <= 65536)
        .ok_or(VolumeError::ProvisioningUncertain(
            "ext4 block geometry is unsupported",
        ))?;
    if (low | (high << 32)).checked_mul(block_size) != Some(capacity) {
        return Err(VolumeError::ProvisioningUncertain(
            "ext4 capacity differs from its intent",
        ));
    }
    Ok(())
}

fn read_u32(bytes: &[u8], start: usize) -> u32 {
    u32::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
    ])
}

#[cfg(test)]
mod tests;
