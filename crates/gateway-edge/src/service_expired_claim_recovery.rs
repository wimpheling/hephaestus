//! Exact-instance takeover for boot-time service cleanup recovery.

use async_trait::async_trait;
use std::time::Duration;

use crate::{
    GatewayServiceIdentity, GatewayServiceInstanceLease, GatewayServiceOwner,
    GatewayServiceOwnershipError,
};

/// Durable ownership operations needed to recover one known expired instance.
#[async_trait]
pub trait GatewayServiceExpiredClaimRecovery: Send + Sync {
    /// Takes over one exact expired instance on the same stable host.
    ///
    /// The adapter must allocate a new fencing epoch, assign `owner`, move the
    /// row to `Stopping`, and return the complete lease. It must reject live,
    /// cleaned, foreign-host, and mismatched identity rows.
    async fn claim_expired_instance(
        &self,
        identity: GatewayServiceIdentity,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;
}
