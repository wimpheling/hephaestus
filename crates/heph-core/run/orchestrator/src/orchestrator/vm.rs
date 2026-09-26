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
}
