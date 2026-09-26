//! Public PAT command, result, error, and service types.

use identity_domain::UserId;
use pat_domain::{
    PersonalAccessToken, PersonalAccessTokenId, PersonalAccessTokenLabel,
    PersonalAccessTokenMetadata, PersonalAccessTokenScope,
};
use sqlx::PgPool;
use time::OffsetDateTime;

/// Parameters for issuing a developer personal access token.
#[derive(Debug, Clone)]
pub struct CreatePersonalAccessToken {
    /// Safe user-visible label.
    pub label: PersonalAccessTokenLabel,
    /// Exact Git operation and optional repository narrowing.
    pub scope: PersonalAccessTokenScope,
    /// Exclusive token expiry, subject to the domain lifetime ceiling.
    pub expires_at: OffsetDateTime,
}

/// Parameters for atomically revoking and replacing a PAT.
#[derive(Debug, Clone)]
pub struct RotatePersonalAccessToken {
    /// Existing PAT owned by the authenticated user.
    pub token_id: PersonalAccessTokenId,
    /// Safe label for the replacement.
    pub label: PersonalAccessTokenLabel,
    /// Exact scope for the replacement; it is not inherited implicitly.
    pub scope: PersonalAccessTokenScope,
    /// Exclusive replacement expiry.
    pub expires_at: OffsetDateTime,
}

/// One newly issued plaintext value and its verifier-free metadata.
///
/// The token is intentionally neither cloneable nor serializable as
/// plaintext. Callers may invoke [`PersonalAccessToken::expose`] only at the
/// one-time delivery boundary.
pub struct IssuedPersonalAccessToken {
    /// Ephemeral plaintext token.
    pub token: PersonalAccessToken,
    /// Safe metadata suitable for a response or listing.
    pub metadata: PersonalAccessTokenMetadata,
}

impl std::fmt::Debug for IssuedPersonalAccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IssuedPersonalAccessToken")
            .field("token", &self.token)
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// Successful token-local Git authentication result.
///
/// Git HTTP must still check the owner's current live authorization for the
/// repository after receiving this result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedPersonalAccessToken {
    /// PAT owner whose live repository authority must be evaluated.
    pub owner_user_id: UserId,
    /// Stable token identifier for audit correlation.
    pub token_id: PersonalAccessTokenId,
}

/// Safe service error that never includes plaintext bearer material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PersonalAccessTokenServiceError {
    /// Requested owner-visible token does not exist.
    #[error("personal access token not found")]
    NotFound,
    /// Credential lookup, verifier, lifecycle, or exact scope did not pass.
    #[error("invalid personal access token credential")]
    InvalidCredential,
    /// The requested lifecycle transition is invalid.
    #[error("invalid personal access token lifecycle transition")]
    InvalidLifecycle,
    /// Issuance input violates the PAT domain contract.
    #[error("invalid personal access token request")]
    InvalidRequest,
    /// Cryptographically secure bearer generation failed.
    #[error("personal access token generation failed")]
    Entropy,
    /// Durable storage did not satisfy the expected contract.
    #[error("personal access token persistence failed")]
    Persistence,
}

/// `PostgreSQL` PAT service usable from authenticated application and trusted
/// Git authentication boundaries.
#[derive(Clone)]
pub struct PostgresPersonalAccessTokenService {
    pub(super) pool: PgPool,
}
