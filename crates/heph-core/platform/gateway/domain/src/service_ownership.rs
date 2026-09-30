//! Durable ownership contracts for long-lived gateway service instances.

use async_trait::async_trait;
use std::{fmt, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::GatewayServiceIdentity;

/// Maximum length of the configured stable host identity.
pub const MAX_SERVICE_OWNER_HOST_BYTES: usize = 200;
/// Maximum number of expired service claims recovered in one bounded pass.
pub const MAX_SERVICE_OWNERSHIP_BATCH: usize = 128;
/// Maximum owner lease duration accepted by the adapter.
pub const MAX_SERVICE_OWNERSHIP_LEASE: Duration = Duration::from_secs(86_400);

/// Lifecycle state persisted for one gateway service launch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayServiceInstanceState {
    /// The durable claim exists and materialization/provisioning may begin.
    Provisioning,
    /// The VM is being started.
    Starting,
    /// Readiness has been established by a later lifecycle slice.
    Ready,
    /// The instance is draining before shutdown or cutover.
    Draining,
    /// The caller owns cleanup of the instance.
    Stopping,
    /// Launch or cleanup failed; the row remains live until cleaned.
    Failed,
    /// Provider and materializer cleanup completed.
    Cleaned,
}

impl GatewayServiceInstanceState {
    /// Returns whether the row still reserves the gateway/revision pair.
    #[must_use]
    pub const fn is_live(self) -> bool {
        !matches!(self, Self::Cleaned)
    }
}

impl fmt::Display for GatewayServiceInstanceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Provisioning => "provisioning",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
            Self::Cleaned => "cleaned",
        })
    }
}

/// Identity of one daemon incarnation holding service ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceOwner {
    /// Stable physical host identity used for same-host recovery.
    pub host_id: String,
    /// Fresh UUID generated for each daemon process incarnation.
    pub owner_uuid: Uuid,
}

impl GatewayServiceOwner {
    /// Creates a validated service owner identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the host identity is empty, oversized, or
    /// contains control/whitespace bytes, or when the owner UUID is nil.
    pub fn new(
        host_id: impl Into<String>,
        owner_uuid: Uuid,
    ) -> Result<Self, GatewayServiceOwnershipError> {
        let host_id = host_id.into();
        let owner = Self {
            host_id,
            owner_uuid,
        };
        owner.validate()?;
        Ok(owner)
    }

    /// Validates public owner fields before they cross a storage boundary.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceOwnershipError::InvalidArgument`] for malformed
    /// host identities or a nil owner UUID.
    pub fn validate(&self) -> Result<(), GatewayServiceOwnershipError> {
        if self.host_id.is_empty()
            || self.host_id.len() > MAX_SERVICE_OWNER_HOST_BYTES
            || self.host_id != self.host_id.trim()
            || self
                .host_id
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            || self.owner_uuid.is_nil()
        {
            return Err(GatewayServiceOwnershipError::InvalidArgument);
        }
        Ok(())
    }
}

/// Lease and immutable identity returned for one service instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceInstanceLease {
    /// Exact launch-attempt identity and gateway/revision pair.
    pub identity: GatewayServiceIdentity,
    /// Host that owns this instance claim.
    pub owner_host_id: String,
    /// Daemon incarnation that owns this claim.
    pub owner_uuid: Uuid,
    /// Monotonic fencing token for this launch attempt.
    pub fencing_token: i64,
    /// Durable lifecycle state.
    pub state: GatewayServiceInstanceState,
    /// Deterministic provider VM identifier.
    pub vm_id: String,
    /// Lease expiry used by recovery.
    pub lease_expires_at: OffsetDateTime,
    /// Last successful owner heartbeat.
    pub heartbeat_at: OffsetDateTime,
}

/// Durable service ownership failures with stale-holder distinctions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceOwnershipError {
    /// Caller supplied an invalid identity or lease duration.
    #[error("invalid gateway service ownership argument")]
    InvalidArgument,
    /// The gateway/revision is not an enabled service declaration or another
    /// non-cleaned instance already reserves it.
    #[error("gateway service ownership conflicts with current durable state")]
    Conflict,
    /// The exact owner, host, fencing token, or unexpired lease is no longer
    /// current.
    #[error("gateway service ownership lease is stale")]
    StaleLease,
    /// Durable storage was unavailable.
    #[error("gateway service ownership is unavailable")]
    Unavailable,
}

/// Provider-neutral durable ownership operations for long-lived services.
#[async_trait]
pub trait GatewayServiceOwnership: Send + Sync {
    /// Claims a fresh provisioning row for an enabled desired/active service
    /// revision. The database allocates the immutable instance identity.
    async fn claim_new(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;

    /// Renews a claim only for its exact owner, host, fence, and live lease.
    async fn renew(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;

    /// Claims expired rows on the same host for bounded cleanup recovery.
    async fn claim_expired(
        &self,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
        limit: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>;

    /// Marks an exact live claim as stopping before external cleanup.
    async fn mark_stopping(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;

    /// Moves an exact provisioning claim into startup ownership.
    async fn mark_starting(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;

    /// Marks an exact starting claim ready after the caller's readiness probe.
    async fn mark_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;

    /// Moves an exact ready non-serving claim into draining.
    async fn mark_draining(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;

    /// Atomically promotes an exact ready candidate to the desired serving
    /// revision, returning the previous active revision when one existed.
    async fn promote_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError>;

    /// Marks an exact stopping claim cleaned after provider/materializer
    /// cleanup has completed successfully.
    async fn mark_cleaned(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError>;
}
