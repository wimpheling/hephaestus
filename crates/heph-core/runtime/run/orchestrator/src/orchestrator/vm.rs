use async_trait::async_trait;
use run_domain::Run;
use vm_trait::{VmError, VmSpec};

/// Builds the non-volume portion of a VM specification for a run.
#[async_trait]
pub trait VmSpecFactory: Send + Sync + 'static {
    /// Creates a VM specification. The orchestrator replaces any disk named
    /// `agent-state` with the currently leased attachment.
    ///
    /// # Errors
    ///
    /// Returns an error when run configuration cannot produce a valid spec.
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError>;

    /// Builds an exact released spec for the complete selected set, including empty.
    ///
    /// The orchestrator installs controlled disks and guest metadata itself.
    /// Factories must explicitly support this profile; there is no scalar or
    /// first-slot fallback. The selection is evidence, not authorization.
    async fn build_with_volumes(
        &self,
        _run: &Run,
        _selections: &volume_domain::RunVolumeSelections,
    ) -> Result<VmSpec, VmError> {
        Err(VmError::InvalidState(
            "complete-set VM specification is unsupported",
        ))
    }
}
