//! Descriptor-relative private filesystem operations with bounded reads.

use super::{JournalError, codec::MAX_RECORD_BYTES};
use rustix::fs::{Mode, OFlags, RenameFlags};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::MetadataExt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}
impl FileIdentity {
    pub fn of(file: &File) -> Result<Self, JournalError> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub fn encode(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.device.to_be_bytes());
        bytes.extend_from_slice(&self.inode.to_be_bytes());
    }
}

pub fn validate_private_file(file: &File) -> Result<(), JournalError> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(JournalError::RecoveryRequired(
            "journal or backing is not a private singly linked inode",
        ));
    }
    Ok(())
}
pub fn validate_directory(file: &File, private: bool) -> Result<(), JournalError> {
    let metadata = file.metadata()?;
    let forbidden = if private { 0o077 } else { 0o022 };
    if !metadata.is_dir()
        || metadata.mode() & forbidden != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(JournalError::RecoveryRequired(
            "journal directory ownership or permissions differ",
        ));
    }
    Ok(())
}
pub fn open_directory(parent: &File, name: &str) -> Result<File, rustix::io::Errno> {
    rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(Into::into)
}
pub fn open_file(parent: &File, name: &str, writable: bool) -> Result<File, rustix::io::Errno> {
    let access = if writable {
        OFlags::RDWR
    } else {
        OFlags::RDONLY
    };
    rustix::fs::openat(
        parent,
        name,
        access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(Into::into)
}
pub fn create_file(parent: &File, name: &str) -> Result<File, rustix::io::Errno> {
    rustix::fs::openat(
        parent,
        name,
        OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map(Into::into)
}
pub fn exists(parent: &File, name: &str) -> Result<bool, JournalError> {
    match rustix::fs::statat(parent, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
pub fn read_record(parent: &File, name: &str) -> Result<Option<Vec<u8>>, JournalError> {
    let file = match open_file(parent, name, false) {
        Ok(file) => file,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => {
            return Err(JournalError::RecoveryRequired(
                "journal record cannot be opened without following links",
            ));
        }
    };
    validate_private_file(&file)?;
    if file.metadata()?.len()
        > u64::try_from(MAX_RECORD_BYTES)
            .map_err(|_| JournalError::RecoveryRequired("record bound overflows"))?
    {
        return Err(JournalError::RecoveryRequired(
            "journal record exceeds its bound",
        ));
    }
    let mut bytes = Vec::new();
    file.take(
        u64::try_from(MAX_RECORD_BYTES + 1)
            .map_err(|_| JournalError::RecoveryRequired("record bound overflows"))?,
    )
    .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(JournalError::RecoveryRequired(
            "journal record changed beyond its bound",
        ));
    }
    Ok(Some(bytes))
}

/// Never overwrites a record. A partial pending file remains evidence of uncertainty.
pub fn publish_record(parent: &File, name: &str, bytes: &[u8]) -> Result<(), JournalError> {
    let pending = format!("{name}.pending");
    let mut file = create_file(parent, &pending).map_err(|_| {
        JournalError::RecoveryRequired("journal pending record already exists or cannot be created")
    })?;
    file.write_all(bytes)?;
    file.sync_all()?;
    rename_new(parent, &pending, parent, name)?;
    parent.sync_all()?;
    Ok(())
}
pub fn rename_new(from: &File, source: &str, to: &File, target: &str) -> Result<(), JournalError> {
    match rustix::fs::renameat_with(from, source, to, target, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::EXIST) => {
            Err(JournalError::Conflict("publication target already exists"))
        }
        Err(error) => Err(error.into()),
    }
}
