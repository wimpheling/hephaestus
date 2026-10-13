//! Descriptor-bound checks within a cooperative trusted host root.
//!
//! No Linux inode-conditional unlink exists: same-UID malicious replacement
//! after the final check is outside this cooperative guarantee.

use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use rustix::fs::{AtFlags, Mode, OFlags};
use std::{
    fs::{self, File, Metadata, Permissions},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

#[cfg(test)]
#[path = "pinned/tests.rs"]
mod tests;

const MAX_ENVELOPE_BYTES: u64 = 4096;

#[derive(Debug)]
pub struct PinError;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}
impl Identity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
        }
    }
}

/// Retains the actual original inode; no later path can reconstruct it.
pub struct Root {
    file: File,
    original: Identity,
}

pub struct Name(String);
impl Name {
    pub(super) fn runtime(
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Self {
        Self(format!(
            "{}-{}.handoff",
            session.as_uuid().hyphenated(),
            generation.get()
        ))
    }
    pub(super) fn git(session: RuntimeSessionId, generation: RuntimeCredentialGeneration) -> Self {
        Self(format!(
            "{}-{}.git-handoff",
            session.as_uuid().hyphenated(),
            generation.get()
        ))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    identity: Identity,
    links: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            identity: Identity::of(metadata),
            links: metadata.nlink(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

pub struct Leaf {
    file: File,
    name: Name,
    original: Stamp,
}

impl Root {
    pub(super) fn initialize(path: &Path) -> Result<Self, PinError> {
        fs::create_dir_all(path).map_err(|_| PinError)?;
        let file: File = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| PinError)?
        .into();
        let metadata = file.metadata().map_err(|_| PinError)?;
        if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(PinError);
        }
        // Constrain the opened original, never a followed named symlink.
        file.set_permissions(Permissions::from_mode(0o700))
            .map_err(|_| PinError)?;
        let original = Identity::of(&file.metadata().map_err(|_| PinError)?);
        let root = Self { file, original };
        root.validate(path)?;
        Ok(root)
    }

    pub(super) fn validate(&self, path: &Path) -> Result<(), PinError> {
        let held = self.file.metadata().map_err(|_| PinError)?;
        let named = fs::symlink_metadata(path).map_err(|_| PinError)?;
        if !held.is_dir()
            || !named.is_dir()
            || named.file_type().is_symlink()
            || held.mode() & 0o7777 != 0o700
            || held.uid() != rustix::process::geteuid().as_raw()
            || Identity::of(&held) != self.original
            || Identity::of(&named) != self.original
        {
            return Err(PinError);
        }
        Ok(())
    }

    pub(super) fn acquire(&self, path: &Path, name: Name) -> Result<Option<Leaf>, PinError> {
        self.validate(path)?;
        let file = match self.open_named(&name) {
            Ok(file) => file,
            Err(rustix::io::Errno::NOENT) => {
                // A racing appearance is uncertainty, not idempotent absence.
                match rustix::fs::statat(&self.file, name.0.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
                    Err(rustix::io::Errno::NOENT) => {
                        self.validate(path)?;
                        return Ok(None);
                    }
                    _ => return Err(PinError),
                }
            }
            Err(_) => return Err(PinError),
        };
        let metadata = file.metadata().map_err(|_| PinError)?;
        self.private_leaf(&metadata)?;
        let leaf = Leaf {
            file,
            name,
            original: Stamp::of(&metadata),
        };
        self.validate(path)?;
        Ok(Some(leaf))
    }

    fn open_named(&self, name: &Name) -> Result<File, rustix::io::Errno> {
        rustix::fs::openat(
            &self.file,
            name.0.as_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(Into::into)
    }

    fn private_leaf(&self, metadata: &Metadata) -> Result<(), PinError> {
        if !metadata.is_file()
            || metadata.dev() != self.original.device
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
            || metadata.uid() != self.original.uid
            || metadata.len() > MAX_ENVELOPE_BYTES
        {
            return Err(PinError);
        }
        Ok(())
    }

    pub(super) fn unlink(&self, path: &Path, leaf: &Leaf) -> Result<(), PinError> {
        self.validate(path)?;
        let held = leaf.file.metadata().map_err(|_| PinError)?;
        self.private_leaf(&held)?;
        let named = self.open_named(&leaf.name).map_err(|_| PinError)?;
        let metadata = named.metadata().map_err(|_| PinError)?;
        self.private_leaf(&metadata)?;
        if Stamp::of(&held) != leaf.original || Stamp::of(&metadata) != leaf.original {
            return Err(PinError);
        }
        self.validate(path)?;
        // Both original descriptors remain held through descriptor-relative removal.
        rustix::fs::unlinkat(&self.file, leaf.name.0.as_str(), AtFlags::empty())
            .map_err(|_| PinError)?;
        self.validate(path)
    }
}

impl Leaf {
    pub(super) fn bytes(&self) -> Result<Vec<u8>, PinError> {
        let mut bytes = Vec::new();
        (&self.file)
            .take(MAX_ENVELOPE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| PinError)?;
        if u64::try_from(bytes.len()).map_err(|_| PinError)? > MAX_ENVELOPE_BYTES
            || Stamp::of(&self.file.metadata().map_err(|_| PinError)?) != self.original
        {
            return Err(PinError);
        }
        Ok(bytes)
    }
}
