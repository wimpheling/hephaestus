//! Read-only ports for durable persistent-gateway service targets.

use crate::GatewayServiceConfig;
use async_trait::async_trait;
use uuid::Uuid;

use crate::GatewayEdgeError;
use crate::{GatewayServiceIdentity, GatewayServiceInstanceKey, GatewayServiceInstanceLease};

/// Maximum number of gateways returned by one service-target page.
pub const MAX_SERVICE_TARGET_PAGE_SIZE: u16 = 128;
/// Maximum number of durable service instances returned by one inventory page.
pub const MAX_SERVICE_INSTANCE_PAGE_SIZE: u16 = 128;

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

/// Bounded stable-UUID page for one daemon host's non-cleaned instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceInstancePage {
    /// Stable host identity; daemon incarnation UUIDs are intentionally ignored.
    pub host_id: String,
    /// Return instances strictly after this immutable instance identity.
    pub after: Option<Uuid>,
    /// Maximum number of instances to return.
    pub limit: u16,
}

impl GatewayServiceInstancePage {
    /// Creates a validated host inventory page.
    ///
    /// # Errors
    ///
    /// Returns a contract error for an invalid host, cursor, or page size.
    pub fn new(
        host_id: impl Into<String>,
        after: Option<Uuid>,
        limit: u16,
    ) -> Result<Self, GatewayEdgeError> {
        let page = Self {
            host_id: host_id.into(),
            after,
            limit,
        };
        page.validate()?;
        Ok(page)
    }

    /// Validates a page assembled by an internal caller.
    ///
    /// # Errors
    ///
    /// Returns a contract error for an invalid host, cursor, or page size.
    pub fn validate(&self) -> Result<(), GatewayEdgeError> {
        if self.host_id.is_empty()
            || self.host_id.len() > crate::MAX_SERVICE_OWNER_HOST_BYTES
            || self.host_id != self.host_id.trim()
            || self
                .host_id
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            || self.limit == 0
            || self.limit > MAX_SERVICE_INSTANCE_PAGE_SIZE
            || self.after.is_some_and(|value| value.is_nil())
        {
            return Err(GatewayEdgeError::Contract(
                "invalid gateway service instance page",
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

/// One bounded page of stable-host service-instance inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceInstancePageResult {
    /// Non-cleaned instances ordered by immutable instance UUID.
    pub instances: Vec<GatewayServiceInstanceLease>,
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

    /// Counts accepted invocations bound to one exact instance and fencing
    /// epoch.  This is the only count safe for deciding when that instance
    /// may finish draining.
    async fn count_accepted_service_invocations_for_instance(
        &self,
        key: GatewayServiceInstanceKey,
    ) -> Result<u64, GatewayEdgeError>;

    /// Looks up one exact durable instance identity, including cleaned rows.
    /// The lookup intentionally does not filter by fencing token so recovery
    /// can observe a newer owner epoch after a claim-expiry race.
    async fn get_service_instance(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayEdgeError>;

    /// Lists all non-cleaned instances owned by one stable host identity.
    /// Expired rows and claims from older daemon incarnations are included.
    async fn list_service_instances(
        &self,
        page: GatewayServiceInstancePage,
    ) -> Result<GatewayServiceInstancePageResult, GatewayEdgeError>;
}
