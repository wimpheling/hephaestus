//! Immutable launch contracts for long-lived gateway services.

use crate::GatewayServiceConfig;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;
use vm_trait::{PrivateHttpServiceSpec, VmMount, VmSpec};

use crate::GatewayEdgeError;

/// Platform-selected maximum number of private service connections.
pub const DEFAULT_SERVICE_MAX_CONNECTIONS: u32 = 32;
/// Platform-selected setup timeout for one private service connection.
pub const DEFAULT_SERVICE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Immutable identity of one persistent service launch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GatewayServiceIdentity {
    /// Unique host-owned launch-attempt identity.
    pub instance_id: Uuid,
    /// Durable gateway identity.
    pub gateway_id: Uuid,
    /// Exact immutable gateway revision identity.
    pub revision_id: Uuid,
}

/// Request for an immutable service launch specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceLaunchRequest {
    /// Exact service launch identity to resolve.
    pub identity: GatewayServiceIdentity,
}

/// Fully resolved service launch and its validated release declaration.
#[derive(Debug, Clone)]
pub struct GatewayServiceLaunch {
    /// Exact launch identity used for labels, VM identity, and materialization.
    pub identity: GatewayServiceIdentity,
    /// Immutable typed service declaration.
    pub service: GatewayServiceConfig,
    /// Provider-ready VM specification with no runtime bearer authority.
    pub spec: VmSpec,
}

/// Verified artifact metadata selected for one immutable service release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceArtifact {
    /// Safe release-relative path.
    pub path: String,
    /// Closed persisted artifact kind.
    pub kind: GatewayServiceArtifactKind,
    /// Exact Unix mode declared by the release.
    pub mode: u32,
    /// SHA-256 digest of the canonical object.
    pub content_hash: [u8; 32],
    /// Exact canonical object length.
    pub size_bytes: u64,
    /// Opaque canonical object-store key.
    pub storage_key: Uuid,
}

/// Closed artifact kinds accepted for a service release tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceArtifactKind {
    /// Guest-executable regular file.
    Executable,
    /// Non-executable regular file.
    File,
    /// Non-executable manifest file.
    Manifest,
}

/// Host-owned materialization port for one persistent service tree.
pub trait GatewayServiceMaterializer: Send + Sync {
    /// Materializes exact release artifacts and parameters for one identity.
    ///
    /// # Errors
    ///
    /// Returns a safe gateway error when materialization fails.
    fn prepare_service(
        &self,
        identity: GatewayServiceIdentity,
        artifacts: &[GatewayServiceArtifact],
        parameters: &Value,
    ) -> Result<Vec<VmMount>, GatewayEdgeError>;

    /// Removes only the exact owned service identity after VM teardown.
    ///
    /// # Errors
    ///
    /// Returns a safe gateway error when cleanup fails.
    fn destroy_service(&self, identity: GatewayServiceIdentity) -> Result<(), GatewayEdgeError>;
}

/// Resolves an immutable published service revision without invocation or
/// runtime-session context.
#[async_trait]
pub trait GatewayServiceLaunchResolver: Send + Sync {
    /// Resolves and materializes one exact service launch.
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError>;

    /// Cleans the exact materialized service identity after provider teardown.
    async fn cleanup_service_launch(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError>;
}

/// Builds the provider-neutral private-service limits selected by the platform.
#[must_use]
pub const fn service_transport_spec(config: &GatewayServiceConfig) -> PrivateHttpServiceSpec {
    PrivateHttpServiceSpec {
        loopback_port: config.loopback_port,
        max_connections: DEFAULT_SERVICE_MAX_CONNECTIONS,
        connect_timeout: DEFAULT_SERVICE_CONNECT_TIMEOUT,
    }
}
