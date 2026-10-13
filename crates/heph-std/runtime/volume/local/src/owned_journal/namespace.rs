//! Exclusive same-volume flock and durably published private namespace.

use super::{
    JournalError, JournalPurpose, MAX_ATTEMPTS, codec,
    filesystem::{self, FileIdentity},
    records,
};
use rustix::fs::{Mode, OFlags};
use std::{fs::File, io::Write};
use uuid::Uuid;

pub struct OwnedJournalLock {
    pub(super) root: File,
    pub(super) lock: File,
    pub(super) purpose: JournalPurpose,
    pub(super) identity: FileIdentity,
}
impl OwnedJournalLock {
    /// Acquires exactly the existing `<volume>.lock` protocol. Private fields
    /// prevent callers from manufacturing a lock witness from an arbitrary file.
    #[cfg(test)]
    pub fn acquire(purpose: JournalPurpose) -> Result<Self, JournalError> {
        use rustix::fs::FlockOperation;
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
        let lock: File = rustix::fs::openat(
            &root,
            format!(".{}.lock", purpose.id()),
            OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?
        .into();
        filesystem::validate_private_file(&lock)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| JournalError::Locked)?;
        root.sync_all()?;
        let identity = FileIdentity::of(&root)?;
        Ok(Self {
            root,
            lock,
            purpose,
            identity,
        })
    }
    pub const fn root(&self) -> &File {
        &self.root
    }
    pub const fn purpose(&self) -> &JournalPurpose {
        &self.purpose
    }
    pub const fn identity(&self) -> FileIdentity {
        self.identity
    }
    /// A formatter child can inherit this duplicate; the flock survives parent
    /// cancellation until the child releases the shared open-file description.
    pub fn child_lock(&self) -> Result<File, JournalError> {
        Ok(self.lock.try_clone()?)
    }
    pub fn validate(&self) -> Result<(), JournalError> {
        filesystem::validate_directory(&self.root, false)?;
        filesystem::validate_private_file(&self.lock)?;
        let path: File = rustix::fs::open(
            self.purpose.root(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?
        .into();
        let linked =
            filesystem::open_file(&self.root, &format!(".{}.lock", self.purpose.id()), false)?;
        if FileIdentity::of(&path)? != self.identity
            || FileIdentity::of(&linked)? != FileIdentity::of(&self.lock)?
        {
            return Err(JournalError::RecoveryRequired(
                "root or lock inode was substituted",
            ));
        }
        Ok(())
    }
}

pub fn open_or_prepare(guard: &OwnedJournalLock) -> Result<File, JournalError> {
    guard.validate()?;
    let name = guard.purpose().namespace_name();
    match filesystem::open_directory(guard.root(), &name) {
        Ok(directory) => {
            filesystem::validate_directory(&directory, true)?;
            Ok(directory)
        }
        Err(rustix::io::Errno::NOENT) => prepare(guard, &name),
        Err(_) => Err(JournalError::RecoveryRequired(
            "existing namespace is not a private directory",
        )),
    }
}
fn prepare(guard: &OwnedJournalLock, name: &str) -> Result<File, JournalError> {
    if filesystem::exists(guard.root(), &guard.purpose().canonical_name())? {
        return Err(JournalError::Conflict(
            "canonical backing exists without a claimed namespace",
        ));
    }
    let prefix = format!("{name}-namespace-attempt");
    let (_, token) = reserve_attempt(
        guard.root(),
        &prefix,
        &codec::digest(guard.purpose().bytes()),
    )?;
    let temporary = format!("{name}-pending-{token}");
    rustix::fs::mkdirat(
        guard.root(),
        &temporary,
        Mode::RUSR | Mode::WUSR | Mode::XUSR,
    )?;
    let directory = filesystem::open_directory(guard.root(), &temporary)?;
    filesystem::validate_directory(&directory, true)?;
    let bytes = records::Birth::encode(
        guard.purpose().bytes(),
        guard.identity(),
        FileIdentity::of(&directory)?,
    )?;
    filesystem::publish_record(&directory, "birth", &bytes)?;
    directory.sync_all()?;
    filesystem::rename_new(guard.root(), &temporary, guard.root(), name)?;
    guard.root().sync_all()?;
    Ok(directory)
}

/// Each exclusive attempt marker consumes one slot even if a write is torn.
/// Unknown markers and unpublished files are retained, never opened for adoption.
pub fn reserve_attempt(
    parent: &File,
    prefix: &str,
    birth: &[u8; 32],
) -> Result<(u8, Uuid), JournalError> {
    for slot in 0..MAX_ATTEMPTS {
        let name = format!("{prefix}-{slot:02}");
        let mut file = match filesystem::create_file(parent, &name) {
            Ok(file) => file,
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => return Err(error.into()),
        };
        let token = Uuid::new_v4();
        let bytes = records::attempt(slot, token, birth)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        parent.sync_all()?;
        return Ok((slot, token));
    }
    Err(JournalError::RecoveryRequired(
        "private journal attempt limit exhausted",
    ))
}

pub fn validate_namespace(
    guard: &OwnedJournalLock,
    directory: &File,
    birth: &records::Birth,
) -> Result<(), JournalError> {
    guard.validate()?;
    filesystem::validate_directory(directory, true)?;
    let linked = filesystem::open_directory(guard.root(), &guard.purpose().namespace_name())
        .map_err(|_| JournalError::RecoveryRequired("claimed namespace was removed or replaced"))?;
    if FileIdentity::of(&linked)? != birth.namespace
        || FileIdentity::of(directory)? != birth.namespace
        || guard.identity() != birth.root
    {
        return Err(JournalError::RecoveryRequired(
            "claimed root or namespace inode differs",
        ));
    }
    Ok(())
}
