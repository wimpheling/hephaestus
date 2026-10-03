//! Positive pre-format evidence and atomic publication of recorded inodes.

use super::{
    FormatState, JournalError, OwnedJournal, RecordedBacking, codec::Kind, filesystem, records,
};

pub(super) struct Phases {
    pub allocated: bool,
    pub published: bool,
    pub format_intent: bool,
}
impl OwnedJournal<'_> {
    pub(super) fn validate_phases(&self) -> Result<Phases, JournalError> {
        if filesystem::exists(&self.directory, "allocation-intent.pending")? {
            return Err(JournalError::RecoveryRequired(
                "allocation intent publication is incomplete",
            ));
        }
        if let Some(bytes) = filesystem::read_record(&self.directory, "allocation-intent")? {
            let inode = self.bound.as_ref().ok_or(JournalError::RecoveryRequired(
                "allocation intent has no recorded inode",
            ))?;
            records::check_phase(Kind::AllocationIntent, &bytes, inode)?;
        }
        let mut present = [false; 3];
        for (index, (name, kind)) in [
            ("allocated", Kind::Allocated),
            ("published", Kind::Published),
            ("format-intent", Kind::FormatIntent),
        ]
        .into_iter()
        .enumerate()
        {
            if filesystem::exists(&self.directory, &format!("{name}.pending"))? {
                return Err(JournalError::RecoveryRequired(
                    "a phase publication is incomplete",
                ));
            }
            if let Some(bytes) = filesystem::read_record(&self.directory, name)? {
                let inode = self.bound.as_ref().ok_or(JournalError::RecoveryRequired(
                    "phase exists without recorded inode ownership",
                ))?;
                records::check_phase(kind, &bytes, inode)?;
                present[index] = true;
            }
        }
        if (present[1] && !present[0]) || (!present[1] && present[2]) {
            return Err(JournalError::RecoveryRequired(
                "journal phases contradict their required predecessors",
            ));
        }
        Ok(Phases {
            allocated: present[0],
            published: present[1],
            format_intent: present[2],
        })
    }
    /// Only complete, contradiction-free durable history can prove no format.
    pub fn format_state(&self) -> Result<FormatState, JournalError> {
        self.validate_namespace()?;
        let phases = self.validate_phases()?;
        if self.bound.is_some() {
            self.recover_recorded()?;
        } else if filesystem::exists(self.guard.root(), &self.guard.purpose().canonical_name())? {
            return Err(JournalError::RecoveryRequired(
                "unrecorded canonical backing cannot prove pre-format state",
            ));
        }
        Ok(if phases.format_intent {
            FormatState::MayHaveStarted
        } else {
            FormatState::NeverStarted
        })
    }
    /// The caller performs allocation. This method only syncs and records its
    /// exact expected length after verifying the already recorded inode.
    pub fn mark_allocated(&self, backing: &RecordedBacking) -> Result<(), JournalError> {
        self.validate_backing(backing)?;
        let phases = self.validate_phases()?;
        if backing.file.metadata()?.len() != self.guard.purpose().capacity() {
            return Err(JournalError::RecoveryRequired(
                "allocated backing length differs from intent",
            ));
        }
        if phases.allocated {
            return Ok(());
        }
        backing.file.sync_all()?;
        filesystem::publish_record(
            &self.directory,
            "allocated",
            &records::phase(Kind::Allocated, &backing.record)?,
        )
    }
    /// Durable allocator intent; exact completed lengths can be synced on actor
    /// recovery without repeating allocation. No formatter has yet been permitted.
    pub fn begin_allocation(&self, backing: &RecordedBacking) -> Result<(), JournalError> {
        self.validate_backing(backing)?;
        let phases = self.validate_phases()?;
        if phases.allocated || phases.published || phases.format_intent {
            return Err(JournalError::RecoveryRequired(
                "allocation is already completed or format may have started",
            ));
        }
        if filesystem::read_record(&self.directory, "allocation-intent")?.is_some() {
            return Ok(());
        }
        filesystem::publish_record(
            &self.directory,
            "allocation-intent",
            &records::phase(Kind::AllocationIntent, &backing.record)?,
        )
    }
    /// Publishes only the known inode, without overwriting any canonical target.
    /// An interrupted rename is reconciled by exact inode identity, not DB state.
    pub fn publish(&self, backing: &RecordedBacking) -> Result<(), JournalError> {
        self.validate_backing(backing)?;
        let phases = self.validate_phases()?;
        if !phases.allocated {
            return Err(JournalError::RecoveryRequired(
                "publication requires durable allocation evidence",
            ));
        }
        if phases.published {
            return Ok(());
        }
        let target = self.guard.purpose().canonical_name();
        if !filesystem::exists(self.guard.root(), &target)? {
            filesystem::rename_new(
                &self.directory,
                &backing.record.name(),
                self.guard.root(),
                &target,
            )?;
        }
        // Revalidate the actual canonical location after rename/recovery.
        let current = self.recover_recorded()?;
        if current.identity() != backing.identity()
            || !filesystem::exists(self.guard.root(), &target)?
        {
            return Err(JournalError::RecoveryRequired(
                "publication did not expose the recorded inode",
            ));
        }
        self.directory.sync_all()?;
        self.guard.root().sync_all()?;
        filesystem::publish_record(
            &self.directory,
            "published",
            &records::phase(Kind::Published, &backing.record)?,
        )
    }
    /// Must return successfully BEFORE starting mkfs. A repeat cannot authorize
    /// another format: even a successful child with a lost result is ambiguous.
    pub fn begin_first_format(&self, backing: &RecordedBacking) -> Result<(), JournalError> {
        self.validate_backing(backing)?;
        let phases = self.validate_phases()?;
        if !phases.allocated || !phases.published || phases.format_intent {
            return Err(JournalError::RecoveryRequired(
                "first format requires allocated published backing and no earlier format intent",
            ));
        }
        filesystem::publish_record(
            &self.directory,
            "format-intent",
            &records::phase(Kind::FormatIntent, &backing.record)?,
        )
    }
}
