use std::sync::Arc;
use vm_trait::{VmId, VmProvider, VmSpec};

/// Optional observable behaviors supported by a provider test fixture.
#[derive(Debug, Clone, Copy)]
pub struct TestCapabilities {
    /// Whether the fixture emits a readiness event after startup.
    pub ready_events: bool,
}

impl Default for TestCapabilities {
    fn default() -> Self {
        Self { ready_events: true }
    }
}

/// Adapter between provider fixtures and the shared contract tests.
pub trait ProviderHarness: Send + Sync {
    /// Returns the provider scoped to this harness.
    fn provider(&self) -> Arc<dyn VmProvider>;

    /// Creates a valid specification that runs until stopped or destroyed.
    fn long_running_spec(&self, id: &str) -> VmSpec;

    /// Creates an optional valid specification with ephemeral user-mode
    /// ingress for providers that implement that trait feature.
    fn ephemeral_ingress_spec(&self, _id: &str) -> Option<VmSpec> {
        None
    }

    /// Describes optional observable behavior available to conformance tests.
    fn capabilities(&self) -> TestCapabilities {
        TestCapabilities::default()
    }

    /// Creates an optional specification whose caller-owned paths can be
    /// snapshotted before lifecycle cleanup.
    fn caller_owned_spec(&self, _id: &str) -> Option<VmSpec> {
        None
    }

    /// Confirms provider-specific resources were released after destruction.
    fn assert_clean(&self, _id: &VmId) {}
}
