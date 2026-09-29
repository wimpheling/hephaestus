//! Exact-instance takeover for boot-time service cleanup recovery.

use async_trait::async_trait;
use std::time::Duration;

use crate::{GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnershipError};

/// Durable ownership operations needed to recover one known expired instance.
#[async_trait]
pub trait GatewayServiceExpiredClaimRecovery: Send + Sync {
    /// Resolves one exact instance behind the gateway ownership barrier.
    ///
    /// Unlike revision-level claim resolution, this includes `Cleaned` rows so
    /// a caller can confirm the durable result of an ambiguous takeover or
    /// cleanup acknowledgement. The lookup is read-only and returns `None`
    /// only when the exact identity is absent.
    async fn resolve_exact_instance(
        &self,
        identity: crate::GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError>;

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
