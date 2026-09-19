//! Read-only ports for durable persistent-gateway service targets.

use async_trait::async_trait;
use gateway_domain::GatewayServiceConfig;
use uuid::Uuid;

use crate::GatewayEdgeError;

/// Maximum number of gateways returned by one service-target page.
pub const MAX_SERVICE_TARGET_PAGE_SIZE: u16 = 128;

/// Bounded stable-UUID cursor request for service targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceTargetPage {
    /// Return gateways strictly after this gateway identity.
    pub after: Option<Uuid>,
    /// Maximum number of gateways to return.
    pub limit: u16,
}

impl GatewayServiceTargetPage {
    /// Creates a validated service-target page request.
    ///
    /// # Errors
    ///
    /// Returns a contract error when the page size is zero or exceeds the
    /// platform bound, or when the cursor is nil.
    pub fn new(after: Option<Uuid>, limit: u16) -> Result<Self, GatewayEdgeError> {
        let page = Self { after, limit };
        page.validate()?;
        Ok(page)
    }

    /// Validates a page value assembled by an internal caller.
    ///
    /// # Errors
    ///
    /// Returns a contract error when the page size is zero or exceeds the
    /// platform bound, or when the cursor is nil.
    pub fn validate(self) -> Result<(), GatewayEdgeError> {
        if self.limit == 0
            || self.limit > MAX_SERVICE_TARGET_PAGE_SIZE
            || self.after.is_some_and(|value| value.is_nil())
        {
            return Err(GatewayEdgeError::Contract(
                "invalid gateway service target page",
            ));
        }
        Ok(())
    }
}

/// One immutable service revision and its publication eligibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceRevisionTarget {
    /// Exact immutable gateway revision identity.
    pub revision_id: Uuid,
    /// Release supplying the service, when the revision still references one.
    pub release_id: Option<Uuid>,
    /// Persisted release publication state, retained to distinguish revoked
    /// candidates from an older published serving revision.
    pub release_state: Option<String>,
    /// Whether this release may be launched by the service resolver.
    pub publication_eligible: bool,
    /// Typed loopback and readiness declaration.
    pub service: GatewayServiceConfig,
}

/// Current service-related target state for one enabled gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceTarget {
    /// Durable gateway identity.
    pub gateway_id: Uuid,
    /// Durable gateway lifecycle, expected to be `enabled` for page results.
    pub lifecycle: String,
    /// Revision currently serving, regardless of whether it is stateless.
    pub active_revision_id: Option<Uuid>,
    /// Latest declared service revision awaiting readiness or activation.
    pub desired_service_revision_id: Option<Uuid>,
    /// Active revision when it is an HTTP service revision.
    pub active_service_revision: Option<GatewayServiceRevisionTarget>,
    /// Desired service revision, including a candidate that is not published.
    pub desired_service_revision: Option<GatewayServiceRevisionTarget>,
}

/// Exact service revision target for an owned instance, including lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceOwnedTarget {
    /// Durable gateway identity.
    pub gateway_id: Uuid,
    /// Current gateway lifecycle, including paused or removed states.
    pub lifecycle: String,
    /// Revision currently serving, if any.
    pub active_revision_id: Option<Uuid>,
    /// Latest desired service revision, if any.
    pub desired_service_revision_id: Option<Uuid>,
    /// Exact immutable service revision used by the owned instance.
    pub revision: GatewayServiceRevisionTarget,
}

/// One bounded page of service target state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceTargetPageResult {
    /// Targets ordered by gateway UUID.
    pub targets: Vec<GatewayServiceTarget>,
    /// Cursor for the next page, when more rows exist.
    pub next_after: Option<Uuid>,
}

/// Read-only gateway service target queries for a lifecycle supervisor.
#[async_trait]
pub trait GatewayServiceTargetStore: Send + Sync {
    /// Lists enabled gateways with a desired service or active service
    /// revision. The cursor is stable and the result is bounded.
    async fn list_service_targets(
        &self,
        page: GatewayServiceTargetPage,
    ) -> Result<GatewayServiceTargetPageResult, GatewayEdgeError>;

    /// Gets one exact service revision even when its gateway is paused,
    /// removed, or no longer points at it, so an owner can drain and clean it.
    async fn get_service_target(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceOwnedTarget>, GatewayEdgeError>;

    /// Counts durable accepted invocations for one exact service revision.
    async fn count_accepted_service_invocations(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<u64, GatewayEdgeError>;
}
