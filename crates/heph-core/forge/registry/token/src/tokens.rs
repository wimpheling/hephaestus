use std::fmt;

use crate::{RegistryTokenClaims, TokenLifetime};

/// A signed bearer token.
pub struct BearerToken(pub(crate) String);

impl BearerToken {
    /// Returns the token only for writing the bearer-token HTTP response.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BearerToken(REDACTED)")
    }
}

/// An issued bearer token and its non-secret response metadata.
pub struct IssuedToken {
    pub(crate) token: BearerToken,
    pub(crate) expires_in: TokenLifetime,
    pub(crate) claims: RegistryTokenClaims,
}

impl IssuedToken {
    /// Returns the signed bearer token for the HTTP response body.
    #[must_use]
    pub const fn token(&self) -> &BearerToken {
        &self.token
    }

    /// Returns the requested token lifetime.
    #[must_use]
    pub const fn expires_in(&self) -> TokenLifetime {
        self.expires_in
    }

    /// Returns the claims that were signed.
    #[must_use]
    pub const fn claims(&self) -> &RegistryTokenClaims {
        &self.claims
    }
}

impl fmt::Debug for IssuedToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedToken")
            .field("token", &"REDACTED")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}
