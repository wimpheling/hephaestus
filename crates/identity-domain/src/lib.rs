//! Provider-neutral authenticated identity values.

use serde::{Deserialize, Serialize};
use serde_json::Value;
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
            trace_id: None,
        }
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
