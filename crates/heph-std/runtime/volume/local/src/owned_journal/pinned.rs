//! Owned acquisition never opens the mutable root path for creation.

use super::{
    JournalError, JournalPurpose, OwnedJournalLock,
    filesystem::{self, FileIdentity},
};
use rustix::fs::{FlockOperation, Mode, OFlags};
use std::fs::File;

impl OwnedJournalLock {
    pub fn acquire_pinned(
        root: &File,
        identity: FileIdentity,
        purpose: JournalPurpose,
        existing: bool,
    ) -> Result<Self, JournalError> {
        filesystem::validate_directory(root, false)?;
        if FileIdentity::of(root)? != identity
            || std::fs::canonicalize(purpose.root())? != purpose.root()
        {
            return Err(JournalError::RecoveryRequired(
                "pinned provider root differs from configured root",
            ));
        }
        let current: File = rustix::fs::open(
            purpose.root(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?
        .into();
        if FileIdentity::of(&current)? != identity {
            return Err(JournalError::RecoveryRequired(
                "provider root was replaced before lock acquisition",
            ));
        }
        // A path replacement after this check cannot redirect CREATE: every write
        // uses the already owned root descriptor, never the replacement directory.
        let lock = if existing {
            filesystem::open_file(root, &format!(".{}.lock", purpose.id()), false)?
        } else {
            rustix::fs::openat(
                root,
                format!(".{}.lock", purpose.id()),
                OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )?
            .into()
        };
        filesystem::validate_private_file(&lock)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| JournalError::Locked)?;
        if !existing {
            root.sync_all()?;
        }
        let result = Self {
            root: root.try_clone()?,
            lock,
            purpose,
            identity,
        };
        result.validate()?;
        Ok(result)
    }
}
