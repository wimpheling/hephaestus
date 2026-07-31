//! Provider-neutral authenticated identity values.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random version 4 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from a UUID.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(UserId, "An immutable internal user identifier.");
identifier!(OrganizationId, "An immutable organization identifier.");
identifier!(RequestId, "A request correlation identifier.");

const IDEMPOTENCY_SEED_DOMAIN: &[u8] = b"hephaestus-mutation-idempotency-seed-v1\0";
const IDEMPOTENCY_ACTOR_DOMAIN: &[u8] = b"hephaestus-mutation-idempotency-actor-v1\0";

/// Produces a one-way seed from an exact operation audience and bounded key.
///
/// The returned digest is safe to pass inward; the raw caller key must never
/// be persisted or logged.
#[must_use]
pub fn mutation_idempotency_seed(audience: &str, idempotency_key: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(IDEMPOTENCY_SEED_DOMAIN);
    update_hash_field(&mut digest, audience.as_bytes());
    update_hash_field(&mut digest, idempotency_key.as_bytes());
    digest.finalize().into()
}

/// Binds a one-way operation seed to the exact authenticated actor.
#[must_use]
pub fn actor_idempotency_id(actor_identity: &[u8], seed: &[u8; 32]) -> RequestId {
    let mut digest = Sha256::new();
    digest.update(IDEMPOTENCY_ACTOR_DOMAIN);
    update_hash_field(&mut digest, actor_identity);
    update_hash_field(&mut digest, seed);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    // RFC 9562 version 8 is reserved for application-defined UUID layouts.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    RequestId::from_uuid(uuid::Uuid::from_bytes(bytes))
}

fn update_hash_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

/// Identity established by verified authentication middleware.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedIdentity {
    /// Immutable internal user identifier.
    pub user_id: UserId,
    /// OIDC issuer that authenticated the request.
    pub issuer: String,
    /// Stable subject at that issuer.
    pub subject: String,
    /// Validated provider claims required by the application.
    pub verified_claims: Value,
    /// Request correlation identifier.
    pub request_id: RequestId,
    /// Opaque logical mutation identifier used only for durable deduplication.
    ///
    /// Non-RPC callers default this to [`Self::request_id`]. Inbound mutation
    /// adapters replace it with a domain-separated identifier derived from the
    /// authenticated actor, exact RPC audience, and bounded idempotency key.
    pub idempotency_id: RequestId,
    /// Optional distributed trace identifier.
    pub trace_id: Option<String>,
}

impl AuthenticatedIdentity {
    /// Creates a request principal after token verification and identity mapping.
    #[must_use]
    pub fn new(
        user_id: UserId,
        issuer: impl Into<String>,
        subject: impl Into<String>,
        verified_claims: Value,
        request_id: RequestId,
    ) -> Self {
        Self {
            user_id,
            issuer: issuer.into(),
            subject: subject.into(),
            verified_claims,
            request_id,
            idempotency_id: request_id,
            trace_id: None,
        }
    }

    /// Replaces the logical mutation identifier without changing client
    /// request provenance.
    #[must_use]
    pub const fn with_idempotency_id(mut self, idempotency_id: RequestId) -> Self {
        self.idempotency_id = idempotency_id;
        self
    }

    /// Adds a distributed trace identifier.
    #[must_use]
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthenticatedIdentity, RequestId, UserId};
    use serde_json::json;

    #[test]
    fn constructs_request_principal() {
        let identity = AuthenticatedIdentity::new(
            UserId::new(),
            "https://issuer.example",
            "subject",
            json!({"email_verified": true}),
            RequestId::new(),
        )
        .with_trace_id("trace");
        assert_eq!(identity.issuer, "https://issuer.example");
        assert_eq!(identity.trace_id.as_deref(), Some("trace"));
    }
}
