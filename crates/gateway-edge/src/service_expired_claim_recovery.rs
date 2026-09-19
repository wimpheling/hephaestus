//! Exact-instance takeover for boot-time service cleanup recovery.

use async_trait::async_trait;
use std::time::Duration;

use crate::{GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnershipError};

/// Durable ownership operations needed to recover one known expired instance.
#[async_trait]
pub trait GatewayServiceExpiredClaimRecovery: Send + Sync {
    /// Takes over one exact expired instance on the same stable host.
    ///
    /// `previous` is an expected-epoch compare-and-swap witness. The adapter
    /// must compare its full immutable identity, owner host/UUID, fencing
    /// token, and deterministic VM ID after taking the gateway and instance
    /// locks. It must allocate exactly the next fencing epoch, assign `owner`,
    /// move the row to `Stopping`, and return the complete lease. It must
    /// reject live, cleaned, foreign-host, newer-epoch, and mismatched rows.
    async fn claim_expired_instance(
        &self,
        previous: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError>;
}
