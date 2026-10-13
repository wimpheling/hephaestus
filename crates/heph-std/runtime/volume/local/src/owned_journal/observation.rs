//! Projection of exact validated host journal facts, never caller supplied bytes.

use super::{JournalError, OwnedJournal, RecordedBacking, codec, filesystem};
use std::os::unix::fs::MetadataExt;
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedBackingObservationWire, OwnedBackingPhase, OwnedFilesystemBirth,
    OwnedProvisioningClaim,
};

impl OwnedJournal<'_> {
    pub fn validate_history(&self, previous: &OwnedBackingObservation) -> Result<(), JournalError> {
        self.validate_namespace()?;
        let phases = self.validate_phases()?;
        let inode = self.bound.as_ref().ok_or(JournalError::RecoveryRequired(
            "recorded database birth lacks its host inode record",
        ))?;
        let fields = previous.fields();
        if fields.root_device != self.birth.root.device
            || fields.root_inode != self.birth.root.inode
            || fields.namespace_device != self.birth.namespace.device
            || fields.namespace_inode != self.birth.namespace.inode
            || fields.backing_device != inode.inode.device
            || fields.backing_inode != inode.inode.inode
            || fields.birth_record_hash != self.birth.hash
            || fields.inode_record_hash != inode.hash
            || fields.owner_uid != self.directory.metadata()?.uid()
            || (!fields.never_format_started && !phases.format_intent)
        {
            return Err(JournalError::RecoveryRequired(
                "host birth contradicts previously committed physical observation",
            ));
        }
        Ok(())
    }
    pub fn observe(
        &self,
        backing: &RecordedBacking,
        claim: &OwnedProvisioningClaim,
        filesystem_birth: Option<OwnedFilesystemBirth>,
    ) -> Result<OwnedBackingObservation, JournalError> {
        self.validate_namespace()?;
        let phases = self.validate_phases()?;
        let current = self.recover_readonly()?;
        if current.record != backing.record
            || current.identity() != backing.identity()
            || claim.purpose.canonical_bytes() != self.guard.purpose().bytes()
            || claim.purpose.birth_generation() != self.guard.purpose().generation()
        {
            return Err(JournalError::Conflict(
                "observation does not match admitted purpose and inode",
            ));
        }
        let phase = match (phases.format_intent, filesystem_birth.is_some()) {
            (true, true) if phases.published => OwnedBackingPhase::Ready,
            (true, false) => OwnedBackingPhase::FormatIntent,
            (false, false) => OwnedBackingPhase::Recorded,
            _ => {
                return Err(JournalError::RecoveryRequired(
                    "filesystem exists without durable format intent",
                ));
            }
        };
        let mut evidence = self.birth.hash.to_vec();
        evidence.extend_from_slice(&backing.record.hash);
        for name in [
            "allocation-intent",
            "allocated",
            "published",
            "format-intent",
        ] {
            if let Some(bytes) = filesystem::read_record(&self.directory, name)? {
                evidence.push(1);
                evidence.extend_from_slice(&bytes);
            } else {
                evidence.push(0);
            }
        }
        OwnedBackingObservation::try_from(OwnedBackingObservationWire {
            id: Uuid::new_v4(),
            purpose_hash: claim.purpose.hash(),
            generation: claim.generation,
            phase,
            root_device: self.birth.root.device,
            root_inode: self.birth.root.inode,
            namespace_device: self.birth.namespace.device,
            namespace_inode: self.birth.namespace.inode,
            owner_uid: current.file.metadata()?.uid(),
            backing_device: backing.record.inode.device,
            backing_inode: backing.record.inode.inode,
            birth_record_hash: self.birth.hash,
            inode_record_hash: backing.record.hash,
            journal_hash: codec::digest(&evidence),
            never_format_started: !phases.format_intent,
            capacity_bytes: current.file.metadata()?.len(),
            filesystem: filesystem_birth,
        })
        .map_err(|_| {
            JournalError::RecoveryRequired("observed journal facts are outside metadata bounds")
        })
    }
    pub fn allocation_complete(&self) -> Result<bool, JournalError> {
        Ok(self.validate_phases()?.allocated)
    }
    pub fn publication_complete(&self) -> Result<bool, JournalError> {
        Ok(self.validate_phases()?.published)
    }
    pub fn format_intent_present(&self) -> Result<bool, JournalError> {
        Ok(self.validate_phases()?.format_intent)
    }
}
