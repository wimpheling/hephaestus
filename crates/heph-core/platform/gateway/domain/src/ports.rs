//! Provider-neutral gateway control-plane ports.

use async_trait::async_trait;
use uuid::Uuid;
use vm_trait::{PrivateMailboxPublication, VmSpec};

use crate::{GatewayEdgeError, GatewayRouteBinding, UiGatewayAuthority};

/// Resolves an authoritative release launch specification for one route.
#[async_trait]
pub trait GatewayReleaseResolver: Send + Sync {
    /// Resolves the released root filesystem, command, and session bootstrap.
    async fn resolve_launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<VmSpec, GatewayEdgeError>;

    /// Persists the exact guest acknowledgement for a launched invocation.
    async fn acknowledge_runtime_authority(
        &self,
        invocation_id: Uuid,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), GatewayEdgeError>;

    /// Releases any host-owned release tree prepared for this invocation.
    async fn cleanup_launch(&self, _: Uuid) -> Result<(), GatewayEdgeError> {
        Ok(())
    }
}

/// Durable audit/session boundary for one invocation.
#[async_trait]
pub trait GatewayInvocationRecorder: Send + Sync {
    /// Records a pre-launch invocation/session and returns its opaque id.
    async fn accepted(
        &self,
        route: &GatewayRouteBinding,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError>;

    /// Records a UI-origin invocation after durable authority rechecking.
    async fn accepted_ui(
        &self,
        route: &GatewayRouteBinding,
        authority: &UiGatewayAuthority,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        let _ = (route, authority, request_id);
        Err(GatewayEdgeError::Contract(
            "UI invocation acceptance is unsupported",
        ))
    }

    /// Records the terminal safe outcome.
    async fn completed(
        &self,
        invocation_id: Uuid,
        outcome: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError>;
}

/// Host-only acceptance port for one bounded mailbox event.
#[async_trait]
pub trait GatewayMailboxPublisher: Send + Sync {
    /// Accepts one bounded candidate or returns a redacted failure.
    async fn publish(
        &self,
        invocation_id: Uuid,
        publication: PrivateMailboxPublication,
    ) -> Result<(), GatewayEdgeError>;
}

/// Persistable terminal invocation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayInvocationOutcome {
    /// Handler returned a bounded response.
    Completed,
    /// Handler/startup failed.
    Failed,
    /// Deadline elapsed.
    TimedOut,
    /// Request was rejected before launch.
    Rejected,
}
