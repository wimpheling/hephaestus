//! Single-writer journal operations under an unforgeable borrowed flock guard.

use super::{
    JournalError, MAX_ATTEMPTS,
    filesystem::{self, FileIdentity},
    namespace::{self, OwnedJournalLock},
    records::{self, Birth, InodeRecord},
};
use std::fs::File;

/// This says nothing about format success; the journal never proves Ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatState {
    NeverStarted,
    MayHaveStarted,
}

pub struct RecordedBacking {
    pub(super) file: File,
    pub(super) record: InodeRecord,
}
impl RecordedBacking {
    /// Descriptor to the exact recorded inode. The trusted future allocator must
    /// check `NeverStarted` before writes and commit format intent before mkfs.
    pub const fn file(&self) -> &File {
        &self.file
    }
    pub const fn identity(&self) -> FileIdentity {
        self.record.inode
    }
}

pub struct OwnedJournal<'a> {
    pub(super) guard: &'a mut OwnedJournalLock,
    pub(super) directory: File,
    pub(super) birth: Birth,
    pub(super) bound: Option<InodeRecord>,
}
impl<'a> OwnedJournal<'a> {
    /// The mutable borrow prevents two journal writers sharing one flock guard.
    pub fn open(guard: &'a mut OwnedJournalLock) -> Result<Self, JournalError> {
        let directory = namespace::open_or_prepare(guard)?;
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
                "canonical backing exists without a recorded owned inode",
            ));
        }
        Ok(result)
    }

    /// Creates only an EMPTY private file. Its birth record is synced before the
    /// descriptor is returned to any allocator or formatter. Failed unrecorded
    /// stages remain unpublished and are never opened for adoption.
    pub fn create_staging(&mut self) -> Result<RecordedBacking, JournalError> {
        self.validate_namespace()?;
        if self.bound.is_some() {
            return Err(JournalError::RecoveryRequired(
                "recorded inode must be recovered before another staging attempt",
            ));
        }
        self.validate_phases()?;
        if filesystem::exists(self.guard.root(), &self.guard.purpose().canonical_name())? {
            return Err(JournalError::Conflict("canonical backing already exists"));
        }
        let (slot, stage) =
            namespace::reserve_attempt(&self.directory, "attempt", &self.birth.hash)?;
        let name = format!("stage-{stage}.raw");
        let file = filesystem::create_file(&self.directory, &name)?;
        filesystem::validate_private_file(&file)?;
        if file.metadata()?.len() != 0 {
            return Err(JournalError::RecoveryRequired(
                "new staging inode is not empty",
            ));
        }
        file.sync_all()?;
        self.directory.sync_all()?;
        let record = InodeRecord::new(slot, stage, FileIdentity::of(&file)?, self.birth.hash)?;
        filesystem::publish_record(
            &self.directory,
            &format!("inode-{slot:02}"),
            &record.encode()?,
        )?;
        self.bound = Some(record.clone());
        Ok(RecordedBacking { file, record })
    }

    /// Observes only exact recorded inode locations. Matching UUIDs or filenames
    /// cannot establish ownership. Publication-before-journal gaps are recoverable.
    pub fn recover_recorded(&self) -> Result<RecordedBacking, JournalError> {
        self.recover_with_access(false)
    }
    pub(super) fn recover_with_access(
        &self,
        readonly: bool,
    ) -> Result<RecordedBacking, JournalError> {
        self.validate_namespace()?;
        let phases = self.validate_phases()?;
        let record = self.bound.as_ref().ok_or(JournalError::RecoveryRequired(
            "no owned inode has been recorded",
        ))?;
        let writable = !readonly && !phases.format_intent;
        let canonical = optional_file(
            self.guard.root(),
            &self.guard.purpose().canonical_name(),
            writable,
        )?;
        let staged = optional_file(&self.directory, &record.name(), writable)?;
        let file = match (canonical, staged) {
            (Some(_), Some(_)) => {
                return Err(JournalError::RecoveryRequired(
                    "recorded backing has contradictory published and staging locations",
                ));
            }
            (Some(file), None) => file,
            (None, Some(file)) if !phases.published => file,
            _ => {
                return Err(JournalError::RecoveryRequired(
                    "recorded owned inode is missing from its durable location",
                ));
            }
        };
        filesystem::validate_private_file(&file)?;
        if FileIdentity::of(&file)? != record.inode {
            return Err(JournalError::RecoveryRequired(
                "backing inode differs from its durable birth record",
            ));
        }
        if phases.allocated && file.metadata()?.len() != self.guard.purpose().capacity() {
            return Err(JournalError::RecoveryRequired(
                "recorded allocated backing length changed",
            ));
        }
        Ok(RecordedBacking {
            file,
            record: record.clone(),
        })
    }
    pub(super) fn validate_namespace(&self) -> Result<(), JournalError> {
        namespace::validate_namespace(self.guard, &self.directory, &self.birth)
    }
    pub(super) fn validate_backing(&self, backing: &RecordedBacking) -> Result<(), JournalError> {
        self.validate_namespace()?;
        filesystem::validate_private_file(&backing.file)?;
        if self.bound.as_ref() != Some(&backing.record)
            || FileIdentity::of(&backing.file)? != backing.record.inode
        {
            return Err(JournalError::RecoveryRequired(
                "backing descriptor does not match this journal",
            ));
        }
        let current = self.recover_recorded()?;
        if current.record != backing.record {
            return Err(JournalError::RecoveryRequired("backing location changed"));
        }
        Ok(())
    }
}

fn optional_file(
    directory: &File,
    name: &str,
    writable: bool,
) -> Result<Option<File>, JournalError> {
    match filesystem::open_file(directory, name, writable) {
        Ok(file) => Ok(Some(file)),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(_) => Err(JournalError::RecoveryRequired(
            "backing cannot be opened without following links",
        )),
    }
}
pub(super) fn load_inode(
    directory: &File,
    birth: &Birth,
) -> Result<Option<InodeRecord>, JournalError> {
    let mut bound = None;
    for slot in 0..MAX_ATTEMPTS {
        if let Some(bytes) = filesystem::read_record(directory, &format!("inode-{slot:02}"))? {
            if filesystem::exists(directory, &format!("inode-{slot:02}.pending"))? {
                return Err(JournalError::RecoveryRequired(
                    "inode birth publication is contradictory",
                ));
            }
            let record = InodeRecord::decode(&bytes, &birth.hash)?;
            if record.slot != slot || bound.is_some() {
                return Err(JournalError::RecoveryRequired(
                    "journal has conflicting inode birth records",
                ));
            }
            let attempt = filesystem::read_record(directory, &format!("attempt-{slot:02}"))?
                .ok_or(JournalError::RecoveryRequired(
                    "inode record lacks a staging attempt",
                ))?;
            records::check_attempt(&attempt, &record)?;
            bound = Some(record);
        }
    }
    Ok(bound)
}
