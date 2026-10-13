//! Recovery opens only preexisting descriptors: no create, rename, sync or format.

use super::{
    JournalError, OwnedJournal, OwnedJournalLock, RecordedBacking, filesystem, journal::load_inode,
    namespace, records::Birth,
};
#[cfg(test)]
use super::{JournalPurpose, filesystem::FileIdentity};
use std::fs::File;

impl OwnedJournalLock {
    #[cfg(test)]
    pub fn acquire_existing(purpose: JournalPurpose) -> Result<Self, JournalError> {
        use rustix::fs::{FlockOperation, Mode, OFlags};
        if std::fs::canonicalize(purpose.root())? != purpose.root() {
            return Err(JournalError::Conflict("configured root is not canonical"));
        }
        let root: File = rustix::fs::open(
            purpose.root(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?
        .into();
        filesystem::validate_directory(&root, false)?;
        let lock = filesystem::open_file(&root, &format!(".{}.lock", purpose.id()), false)?;
        filesystem::validate_private_file(&lock)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| JournalError::Locked)?;
        let identity = FileIdentity::of(&root)?;
        Ok(Self {
            root,
            lock,
            purpose,
            identity,
        })
    }
}
impl<'a> OwnedJournal<'a> {
    pub fn open_existing(guard: &'a mut OwnedJournalLock) -> Result<Self, JournalError> {
        guard.validate()?;
        let directory =
            filesystem::open_directory(guard.root(), &guard.purpose().namespace_name())?;
        if filesystem::exists(&directory, "birth.pending")? {
            return Err(JournalError::RecoveryRequired(
                "birth marker publication is contradictory",
            ));
        }
        let bytes = filesystem::read_record(&directory, "birth")?.ok_or(
            JournalError::RecoveryRequired("claimed namespace lacks its birth marker"),
        )?;
        let birth = Birth::decode(&bytes, guard.purpose().bytes())?;
        namespace::validate_namespace(guard, &directory, &birth)?;
        let bound = load_inode(&directory, &birth)?;
        let result = Self {
            guard,
            directory,
            birth,
            bound,
        };
        result.validate_phases()?;
        if result.bound.is_none()
            && filesystem::exists(
                result.guard.root(),
                &result.guard.purpose().canonical_name(),
            )?
        {
            return Err(JournalError::Conflict(
                "canonical backing has no recorded ownership",
            ));
        }
        Ok(result)
    }
    pub fn recover_readonly(&self) -> Result<RecordedBacking, JournalError> {
        self.recover_with_access(true)
    }
    pub fn child_lock(&self) -> Result<File, JournalError> {
        self.guard.child_lock()
    }
    pub const fn has_recorded_inode(&self) -> bool {
        self.bound.is_some()
    }
}
