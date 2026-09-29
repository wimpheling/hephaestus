//! Strict Docker Distribution-compatible registry bearer tokens.
//!
//! This crate deliberately owns neither caller authentication nor registry
//! namespace authorization. The HTTP and identity adapters parse a caller,
//! resolve live ownership, and inject an [`AuthorizationDecision`]. This
//! crate then intersects that decision with a strictly parsed token request.

const MAX_SERVICE_LENGTH: usize = 255;
const MAX_REPOSITORY_LENGTH: usize = 255;
const MAX_CLAIM_TEXT_LENGTH: usize = 512;
const MAX_KEY_ID_LENGTH: usize = 128;
const MIN_HMAC_SECRET_LENGTH: usize = 32;
const MAX_TOKEN_LIFETIME_SECONDS: u64 = 900;

mod claims;
mod errors;
mod keys;
mod request;
mod scope;
mod service;
mod tokens;
mod validation;

pub use claims::{RegistryAccess, RegistryTokenClaims, TokenIssuer, TokenSubject};
pub use errors::RegistryTokenError;
pub use keys::{KeyId, SigningKey, TokenLifetime, UnixTimestamp, VerificationKey};
pub use request::{AuthorizationDecision, ScopeRequest};
pub use scope::{
    RegistryAction, RegistryService, RepositoryActions, RepositoryName, RepositoryScope,
};
pub use service::{RegistryTokenIssuer, RegistryTokenVerifier};
pub use tokens::{BearerToken, IssuedToken};

#[cfg(test)]
#[path = "tests.rs"]
mod token_tests;

#[cfg(test)]
mod tests {
    use super::{AuthorizationDecision, token_tests};

    #[test]
    fn token_debug_is_redacted() {
        let issued = token_tests::issuer()
            .issue(
                token_tests::subject(),
                &token_tests::request("repository:platform/builders/rust-ubuntu:pull"),
                &AuthorizationDecision::deny_all(),
                token_tests::NOW,
            )
            .expect("token");
        let token = issued.token().as_str();
        assert!(!format!("{issued:?}").contains(token));
        assert!(!format!("{:?}", issued.token()).contains(token));
        assert!(format!("{issued:?}").contains("REDACTED"));
    }
}
