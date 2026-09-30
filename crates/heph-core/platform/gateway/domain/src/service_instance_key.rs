//! Exact fenced identity of one persistent gateway service instance.

use crate::GatewayServiceIdentity;

/// Exact in-memory service key, including the durable fencing token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GatewayServiceInstanceKey {
    /// Immutable service launch identity.
    pub identity: GatewayServiceIdentity,
    /// Positive durable ownership fence.
    pub fencing_token: i64,
}

impl GatewayServiceInstanceKey {
    /// Returns whether all identity components and the fencing token are valid.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        !self.identity.instance_id.is_nil()
            && !self.identity.gateway_id.is_nil()
            && !self.identity.revision_id.is_nil()
            && self.fencing_token > 0
    }
}
