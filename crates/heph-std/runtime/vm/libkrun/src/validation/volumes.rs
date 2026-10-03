use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
};
use vm_trait::{VmError, VmGuestVolume};

use super::{PreparedDisk, helpers::provider_io};

/// Associates each named attachment with the filesystem on its exact raw disk.
/// The provider's canonical private path is read without following a final link.
pub fn validate_named_disk_files(
    volumes: &[VmGuestVolume],
    disks: &[PreparedDisk],
) -> Result<(), VmError> {
    for volume in volumes {
        let mut selected = disks.iter().filter(|disk| disk.id == volume.disk_id());
        let disk = selected.next().ok_or_else(|| invalid("disk unavailable"))?;
        if selected.next().is_some() {
            return Err(invalid("disk identity is ambiguous"));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&disk.path)
            .map_err(provider_io)?;
        if !file.metadata().map_err(provider_io)?.is_file() {
            return Err(invalid("named raw disk must be a regular file"));
        }
        let mut superblock = [0_u8; 120];
        file.seek(SeekFrom::Start(1024)).map_err(provider_io)?;
        file.read_exact(&mut superblock).map_err(provider_io)?;
        if superblock[56..58] != [0x53, 0xef]
            || superblock[104..120] != *volume.filesystem_uuid().as_bytes()
        {
            return Err(invalid("named disk ext4 UUID differs from its attachment"));
        }
    }
    Ok(())
}

fn invalid(reason: &str) -> VmError {
    VmError::InvalidSpec {
        field: "guest_volumes".to_owned(),
        reason: reason.to_owned(),
    }
}
