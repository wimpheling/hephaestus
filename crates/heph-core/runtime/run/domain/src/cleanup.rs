//! Canonical cleanup values; durable worker persistence supplies their proof.

mod host;
mod vm;

pub use host::RunCleanupHostId;

pub use vm::{
    RunCleanupReceipt, RunCleanupVmObservation, RunCleanupVmObservationKind, RunCleanupVmTarget,
    RunCleanupVmTargetKind,
};

use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, LeaseId, RunId, VolumeId};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Maximum complete lease set accepted by the initial runtime profile.
pub const MAX_RUN_CLEANUP_LEASES: usize = 32;

/// A malformed cleanup value or a mismatched target, rather than physical proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunCleanupError {
    /// A stable run, instance, revision, lease, or volume ID is nil.
    #[error("cleanup identity is nil")]
    InvalidIdentity,
    /// The provider identity is empty, excessive, or contains unsafe characters.
    #[error("cleanup provider identity is invalid")]
    InvalidProviderIdentity,
    /// The historical VM identity has not been authoritatively resolved.
    #[error("historical VM identity remains unresolved")]
    UnresolvedVmIdentity,
    /// A cleanup generation is outside the positive `PostgreSQL` bigint range.
    #[error("cleanup generation is invalid")]
    InvalidGeneration,
    /// A lease fence is not positive.
    #[error("cleanup lease fence is not positive")]
    InvalidLeaseFence,
    /// The complete set exceeds the bounded runtime contract.
    #[error("cleanup lease set is too large")]
    TooManyLeases,
    /// Two entries reuse one lease identity.
    #[error("cleanup lease identity is repeated")]
    DuplicateLease,
    /// Two entries reuse one exclusive resource.
    #[error("cleanup resource identity is repeated")]
    DuplicateVolume,
    /// An entry belongs to a different run.
    #[error("cleanup lease belongs to another run")]
    LeaseRunMismatch,
    /// A lease or observation belongs to a different provider host scope.
    #[error("cleanup provider host scope does not match the target")]
    HostMismatch,
    /// The observed VM differs from the exact target.
    #[error("cleanup VM observation does not match the target")]
    VmMismatch,
    /// The receipt differs from the current immutable cleanup target.
    #[error("cleanup receipt does not match the target")]
    TargetMismatch,
}

/// Exact identity, host, and fence of one lease in the canonical complete set.
///
/// Construction validates shape. A repository must verify it against persisted
/// leases while closing acquisition; caller-provided values establish no proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupLeaseFence {
    run_id: RunId,
    lease_id: LeaseId,
    volume_id: VolumeId,
    host_id: String,
    fencing_token: i64,
}

impl RunCleanupLeaseFence {
    /// Constructs a bounded exact lease identity.
    ///
    /// # Errors
    ///
    /// Rejects nil IDs, unsafe host identifiers, or nonpositive fences.
    pub fn new(
        run_id: RunId,
        lease_id: LeaseId,
        volume_id: VolumeId,
        host_id: String,
        fencing_token: i64,
    ) -> Result<Self, RunCleanupError> {
        if run_id.as_uuid().is_nil() || lease_id.as_uuid().is_nil() || volume_id.as_uuid().is_nil()
        {
            return Err(RunCleanupError::InvalidIdentity);
        }
        validate_provider_identity(&host_id)?;
        if fencing_token <= 0 {
            return Err(RunCleanupError::InvalidLeaseFence);
        }
        Ok(Self {
            run_id,
            lease_id,
            volume_id,
            host_id,
            fencing_token,
        })
    }

    /// Returns the exact owning run.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Returns the stable lease identity.
    #[must_use]
    pub const fn lease_id(&self) -> LeaseId {
        self.lease_id
    }

    /// Returns the exclusively leased resource.
    #[must_use]
    pub const fn volume_id(&self) -> VolumeId {
        self.volume_id
    }

    /// Returns the provider host holding the fence.
    #[must_use]
    pub fn host_id(&self) -> &str {
        &self.host_id
    }

    /// Returns the exact monotonic lease fence.
    #[must_use]
    pub const fn fencing_token(&self) -> i64 {
        self.fencing_token
    }
}

/// Immutable snapshot of one closed acquisition boundary and its entire set.
///
/// The generation and digest include the exact VM target and every sorted
/// lease/volume/host/fence. An empty set is explicit data, never inferred from
/// an absent in-memory attachment. Persistence must supply the actual set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupTarget {
    run_id: RunId,
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    vm_target: RunCleanupVmTarget,
    generation: u64,
    leases: Vec<RunCleanupLeaseFence>,
    digest: [u8; 32],
}

impl RunCleanupTarget {
    /// Normalizes and validates a complete immutable cleanup snapshot.
    ///
    /// # Errors
    ///
    /// Rejects nil identities, invalid generations, excess/reused leases or
    /// resources, leases belonging to another run, or lease hosts differing
    /// from the exact VM host. Unresolved historical scope retains its set.
    pub fn new(
        run_id: RunId,
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        vm_target: RunCleanupVmTarget,
        generation: u64,
        mut leases: Vec<RunCleanupLeaseFence>,
    ) -> Result<Self, RunCleanupError> {
        if run_id.as_uuid().is_nil()
            || instance_id.as_uuid().is_nil()
            || revision_id.as_uuid().is_nil()
        {
            return Err(RunCleanupError::InvalidIdentity);
        }
        if generation == 0 || i64::try_from(generation).is_err() {
            return Err(RunCleanupError::InvalidGeneration);
        }
        if leases.len() > MAX_RUN_CLEANUP_LEASES {
            return Err(RunCleanupError::TooManyLeases);
        }
        leases.sort_unstable_by_key(RunCleanupLeaseFence::volume_id);
        let mut lease_ids = BTreeSet::new();
        let mut volumes = BTreeSet::new();
        for lease in &leases {
            if lease.run_id != run_id {
                return Err(RunCleanupError::LeaseRunMismatch);
            }
            if vm_target
                .host()
                .is_some_and(|host| host.host_id() != lease.host_id)
            {
                return Err(RunCleanupError::HostMismatch);
            }
            if !lease_ids.insert(lease.lease_id) {
                return Err(RunCleanupError::DuplicateLease);
            }
            if !volumes.insert(lease.volume_id) {
                return Err(RunCleanupError::DuplicateVolume);
            }
        }
        let mut target = Self {
            run_id,
            instance_id,
            revision_id,
            vm_target,
            generation,
            leases,
            digest: [0; 32],
        };
        target.digest = target.compute_digest();
        Ok(target)
    }

    /// Returns the exact run.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Returns the consumer instance.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }

    /// Returns the selected immutable revision.
    #[must_use]
    pub const fn revision_id(&self) -> AgentInstanceRevisionId {
        self.revision_id
    }

    /// Returns the persisted or planned VM identity, or explicit uncertainty.
    #[must_use]
    pub const fn vm_target(&self) -> &RunCleanupVmTarget {
        &self.vm_target
    }

    /// Returns the positive acquisition-closure generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the complete canonical set sorted by resource identity.
    #[must_use]
    pub fn leases(&self) -> &[RunCleanupLeaseFence] {
        &self.leases
    }

    /// Returns the versioned digest of the exact immutable target.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    fn compute_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"heph.run.cleanup-target.v1\0");
        digest.update(self.run_id.as_uuid().as_bytes());
        digest.update(self.instance_id.as_uuid().as_bytes());
        digest.update(self.revision_id.as_uuid().as_bytes());
        digest.update(self.generation.to_be_bytes());
        digest.update([match self.vm_target.kind() {
            RunCleanupVmTargetKind::Persisted => 1,
            RunCleanupVmTargetKind::Planned => 2,
            RunCleanupVmTargetKind::HistoricalUnresolved => 3,
        }]);
        hash_identifier(
            &mut digest,
            self.vm_target
                .host()
                .map_or("", RunCleanupHostId::provider_namespace),
        );
        hash_identifier(
            &mut digest,
            self.vm_target.host().map_or("", RunCleanupHostId::host_id),
        );
        hash_identifier(
            &mut digest,
            self.vm_target.vm_id().map_or("", |id| id.0.as_str()),
        );
        digest.update([u8::try_from(self.leases.len()).expect("cleanup set is bounded by 32")]);
        for lease in &self.leases {
            digest.update(lease.lease_id.as_uuid().as_bytes());
            digest.update(lease.volume_id.as_uuid().as_bytes());
            hash_identifier(&mut digest, &lease.host_id);
            digest.update(lease.fencing_token.to_be_bytes());
        }
        digest.finalize().into()
    }
}

pub fn validate_provider_identity(value: &str) -> Result<(), RunCleanupError> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(RunCleanupError::InvalidProviderIdentity);
    }
    Ok(())
}

fn hash_identifier(digest: &mut Sha256, value: &str) {
    // All identities are bounded by construction; u16 gives stable encoding
    // independent of the worker's pointer width.
    let length = u16::try_from(value.len()).expect("cleanup identifiers are bounded by 128 bytes");
    digest.update(length.to_be_bytes());
    digest.update(value.as_bytes());
}

#[cfg(test)]
mod tests;
