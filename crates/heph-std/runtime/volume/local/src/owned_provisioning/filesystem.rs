//! Exact descriptor-based initial ext4 birth and readonly consistency proof.

use crate::{backing, invalid_backing};
use std::{
    fs::File,
    os::{fd::AsRawFd, unix::fs::FileExt},
    path::Path,
};
use tokio::process::Command;
use uuid::Uuid;
use volume_trait::{OwnedFilesystemBirth, VolumeError};

pub fn descriptor(file: &File) -> String {
    format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd())
}
pub fn locked_command(executable: &Path, lock: File) -> Command {
    let mut command = Command::new(executable);
    // Shared flock survives future cancellation until this actual child exits.
    command.stdin(std::process::Stdio::from(lock));
    command.kill_on_drop(true);
    command
}
pub fn no_unjournaled_format(file: &File) -> Result<(), VolumeError> {
    if file.metadata().map_err(backing)?.len() < 2048 {
        return Ok(());
    }
    let mut magic = [0; 2];
    file.read_exact_at(&mut magic, 1024 + 56).map_err(backing)?;
    if magic == [0x53, 0xef] {
        return Err(VolumeError::ProvisioningUncertain(
            "filesystem bytes contradict never-format journal",
        ));
    }
    Ok(())
}
pub async fn prove(
    file: &File,
    lock: File,
    capacity: u64,
    uuid: Uuid,
    mkfs: &Path,
) -> Result<OwnedFilesystemBirth, VolumeError> {
    let birth = superblock(file, capacity, uuid)?;
    let executable = mkfs
        .parent()
        .ok_or_else(|| invalid_backing("mkfs.ext4 must have a parent directory"))?
        .join("e2fsck");
    let status = locked_command(&executable, lock)
        .args(["-f", "-n"])
        .arg(descriptor(file))
        .status()
        .await
        .map_err(backing)?;
    if !status.success() {
        return Err(VolumeError::ProvisioningUncertain(
            "readonly filesystem consistency was not proven",
        ));
    }
    // Recheck after the child; readonly proof cannot authorize a different inode/geometry.
    if superblock(file, capacity, uuid)? != birth {
        return Err(VolumeError::IntentConflict);
    }
    Ok(birth)
}
fn superblock(file: &File, capacity: u64, uuid: Uuid) -> Result<OwnedFilesystemBirth, VolumeError> {
    let mut bytes = [0_u8; 1024];
    if file.metadata().map_err(backing)?.len() != capacity {
        return Err(VolumeError::ProvisioningUncertain(
            "owned backing capacity differs",
        ));
    }
    file.read_exact_at(&mut bytes, 1024).map_err(backing)?;
    let features = read_u32(&bytes, 96);
    let count = u64::from(read_u32(&bytes, 4))
        | if features & 0x80 != 0 {
            u64::from(read_u32(&bytes, 336)) << 32
        } else {
            0
        };
    if bytes[56..58] != [0x53, 0xef]
        || bytes[104..120] != *uuid.as_bytes()
        || features & 0x40 == 0
        || read_u32(&bytes, 24) != 2
        || count.checked_mul(4096) != Some(capacity)
        || u16::from_le_bytes([bytes[58], bytes[59]]) != 1
        || features & 0x4 != 0
    {
        return Err(VolumeError::ProvisioningUncertain(
            "owned initial ext4 birth is dirty or differs from intent",
        ));
    }
    Ok(OwnedFilesystemBirth {
        filesystem_uuid: uuid,
        block_size: 4096,
        block_count: count,
        clean: true,
    })
}
fn read_u32(bytes: &[u8], start: usize) -> u32 {
    u32::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
    ])
}
