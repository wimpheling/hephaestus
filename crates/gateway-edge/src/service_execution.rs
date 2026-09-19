//! Durable execution-target lookup for already accepted gateway invocations.

use async_trait::async_trait;
use std::time::Duration;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{GatewayServiceInstanceKey, GatewayServiceOwner};

/// The remaining authority budget returned for one persistent-service request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceAuthorityBudget {
    /// Exact fenced service instance admitted for the invocation.
    pub instance: GatewayServiceInstanceKey,
    /// Durable host-mediated session expiry from `PostgreSQL`.
    pub expires_at: OffsetDateTime,
    /// Conservative remaining duration after accounting for lookup latency.
    pub remaining: Duration,
}

/// Explicit execution target for one already accepted invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayExecutionTarget {
    /// The immutable invocation is a one-request `http.v1` handler.
    Stateless,
    /// The immutable invocation is bound to a live persistent service instance.
    Service(GatewayServiceAuthorityBudget),
}

/// Redacted failures from the durable execution-target authority boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayExecutionTargetError {
    /// The caller supplied malformed identity or owner data.
    #[error("invalid gateway execution target request")]
    InvalidArgument,
    /// The exact accepted invocation is unavailable for execution.
    #[error("gateway execution target is unavailable")]
    Unavailable,
}

impl From<GatewayExecutionTargetError> for crate::GatewayEdgeError {
    fn from(_error: GatewayExecutionTargetError) -> Self {
        Self::HandlerUnavailable
    }
}

/// Resolves the immutable target and current host authority for one request.
#[async_trait]
pub trait GatewayExecutionTargetResolver: Send + Sync {
    /// Resolves an accepted invocation without retaining a database transaction
    /// across the later HTTP exchange.
    async fn resolve_execution_target(
        &self,
        invocation_id: Uuid,
        route_id: Uuid,
        revision_id: Uuid,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayExecutionTarget, GatewayExecutionTargetError>;
}
