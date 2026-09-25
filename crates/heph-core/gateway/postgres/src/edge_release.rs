//! Public contracts for materialized gateway release trees.

use gateway_domain::GatewayEdgeError;
use uuid::Uuid;
use vm_trait::VmMount;

/// Verified artifact metadata selected for one immutable gateway release.
#[derive(Debug, Clone)]
pub struct GatewayReleaseArtifact {
    /// Safe release-relative path.
    pub path: String,
    /// Closed persisted artifact kind.
    pub kind: GatewayReleaseArtifactKind,
    /// Exact Unix mode declared by the release.
    pub mode: u32,
    /// SHA-256 digest of the canonical object.
    pub content_hash: [u8; 32],
    /// Exact canonical object length.
    pub size_bytes: u64,
    /// Opaque canonical object-store key.
    pub storage_key: Uuid,
}

/// Closed artifact kinds accepted for a gateway release tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayReleaseArtifactKind {
    /// Guest-executable regular file.
    Executable,
    /// Non-executable regular file.
    File,
    /// Non-executable manifest file.
    Manifest,
}

/// Host-owned lifecycle boundary for a materialized gateway `/release` tree.
///
/// The resolver supplies only exact persisted metadata. Implementations own
/// object-store I/O, safe filesystem construction, and cleanup after the VM
/// is destroyed.
pub trait GatewayReleaseMaterializer: Send + Sync {
    /// Builds fresh read-only release and parameter mounts for this invocation.
    ///
    /// # Errors
    ///
    /// Returns a safe edge error when the exact release tree cannot be built.
    fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[GatewayReleaseArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, GatewayEdgeError>;
    /// Removes the materialized tree after provider cleanup.
    ///
    /// # Errors
    ///
    /// Returns a safe edge error when the release tree cannot be removed.
    fn destroy(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError>;
}
