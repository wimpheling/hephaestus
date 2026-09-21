//! Serialized read-side resolution for ambiguous service claims.

use async_trait::async_trait;
use uuid::Uuid;

use crate::{GatewayServiceInstanceLease, GatewayServiceOwnershipError};

/// Read-side barrier for resolving whether a service claim committed.
///
/// Implementations must serialize this lookup with the same gateway-row lock
/// used by claim creation before taking the fresh read snapshot. Inventory
/// results without that barrier cannot distinguish an in-flight claim from a
/// rejected claim.
#[async_trait]
pub trait GatewayServiceClaimResolutionStore: Send + Sync {
    /// Resolves the current non-cleaned claim for one exact gateway/revision.
    ///
    /// The result intentionally ignores owner, fencing token, and expiry so a
    /// recovery caller can observe the claim that requires reconciliation.
    /// Missing gateways and missing or cleaned claims return `Ok(None)` only
    /// after the implementation has completed its serialization barrier.
    async fn resolve_revision_claim(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>;
}
