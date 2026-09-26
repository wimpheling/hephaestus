use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    BrokeredEgressError, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName,
    MAX_HEADER_PREFIX_BYTES, PLACEHOLDER_PREFIX,
};

/// The only supported substitution locations. Body, query, and arbitrary
/// matching are deliberately absent from this closed vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HttpInjectionLocation {
    /// Replace a complete outbound header value equal to the placeholder.
    OutboundHeaderValue {
        /// Normalized header selected for complete-value replacement.
        header: HeaderName,
    },
    /// Replace a placeholder following one fixed outbound header prefix.
    OutboundHeaderPrefix {
        /// Normalized header selected for prefix replacement.
        header: HeaderName,
        /// Exact non-secret prefix preserved before the replacement.
        prefix: String,
    },
    /// Replace a verified inbound header before VM delivery.
    InboundGatewayHeader {
        /// Normalized inbound header selected for constant-time verification.
        header: HeaderName,
    },
}

impl HttpInjectionLocation {
    /// Validates a location's bounded prefix and returns its direction.
    ///
    /// # Errors
    ///
    /// Returns [`BrokeredEgressError::InvalidHeaderPrefix`] when a prefix is
    /// too large, non-ASCII, or contains line breaks.
    pub fn normalized(self) -> Result<Self, BrokeredEgressError> {
        if let Self::OutboundHeaderPrefix { prefix, .. } = &self {
            if prefix.len() > MAX_HEADER_PREFIX_BYTES
                || !prefix.is_ascii()
                || prefix.contains(['\r', '\n'])
            {
                return Err(BrokeredEgressError::InvalidHeaderPrefix);
            }
        }
        Ok(self)
    }
    /// Returns whether this is an egress or ingress rule.
    #[must_use]
    pub const fn direction(&self) -> InjectionDirection {
        match self {
            Self::InboundGatewayHeader { .. } => InjectionDirection::Inbound,
            Self::OutboundHeaderValue { .. } | Self::OutboundHeaderPrefix { .. } => {
                InjectionDirection::Outbound
            }
        }
    }
}

/// Direction of one substitution rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionDirection {
    /// VM to HTTPS upstream.
    Outbound,
    /// Public gateway to VM.
    Inbound,
}

/// One immutable, exact authority rule.
///
/// `gateway_route_id` is required only for inbound rules and forbidden for
/// outbound rules. This prevents a rule from silently applying outside its
/// declared workload/route boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokeredSecretRule {
    /// Stable immutable rule identity.
    pub id: BrokeredSecretRuleId,
    /// Immutable secret binding UUID.
    pub binding_id: Uuid,
    /// Immutable agent revision UUID that owns the binding.
    pub instance_revision_id: Uuid,
    /// Exact immutable selected secret version UUID.
    pub secret_version_id: Uuid,
    /// Exact verified HTTPS origin for outbound rules; absent for inbound.
    pub destination: Option<ExactHttpsOrigin>,
    /// Closed bounded header location.
    pub location: HttpInjectionLocation,
    /// Exact immutable gateway route UUID for inbound rules only.
    pub gateway_route_id: Option<Uuid>,
}

impl BrokeredSecretRule {
    /// Validates cross-field authority invariants.
    ///
    /// # Errors
    ///
    /// Returns [`BrokeredEgressError::AmbiguousRuleScope`] when an outbound
    /// rule is route-bound or an inbound rule does not have an exact route.
    pub fn normalized(mut self) -> Result<Self, BrokeredEgressError> {
        self.location = self.location.normalized()?;
        match self.location.direction() {
            InjectionDirection::Outbound
                if self.destination.is_some() && self.gateway_route_id.is_none() =>
            {
                Ok(self)
            }
            InjectionDirection::Inbound
                if self.destination.is_none() && self.gateway_route_id.is_some() =>
            {
                Ok(self)
            }
            _ => Err(BrokeredEgressError::AmbiguousRuleScope),
        }
    }
    /// Derives the stable VM-visible placeholder. This is an identifier, not a
    /// bearer credential and cannot resolve any secret without this rule.
    #[must_use]
    pub fn placeholder(&self) -> String {
        format!("{PLACEHOLDER_PREFIX}{}", self.id)
    }
    /// Returns a canonical, non-secret hash for immutable snapshots/audit.
    ///
    /// # Panics
    ///
    /// Panics only if serializing these fixed, string/UUID-only domain values
    /// unexpectedly fails.
    #[must_use]
    pub fn normalized_hash(&self) -> [u8; 32] {
        let encoded = serde_json::to_vec(self).expect("brokered rule serialization is infallible");
        Sha256::digest(encoded).into()
    }
}
