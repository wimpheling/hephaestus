//! Error values for PAT construction and token-local authorization.

/// PAT construction and lifecycle validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PersonalAccessTokenError {
    /// Plaintext token envelope is not canonical version 1 syntax.
    #[error("invalid personal access token")]
    InvalidToken,
    /// Safe user-visible label is malformed.
    #[error("invalid personal access token label")]
    InvalidLabel,
    /// No Git operation was selected.
    #[error("personal access token scope must contain an operation")]
    EmptyScope,
    /// Exact repository narrowing is empty or exceeds its bound.
    #[error("invalid personal access token repository restrictions")]
    InvalidRepositoryRestrictions,
    /// Expiry is not after issuance or exceeds the hard lifetime ceiling.
    #[error("invalid personal access token lifetime")]
    InvalidLifetime,
    /// A lifecycle mutation targeted an inactive token.
    #[error("personal access token is inactive")]
    InactiveToken,
    /// Last-used metadata would move backwards.
    #[error("personal access token last-used timestamp is not monotonic")]
    NonMonotonicLastUse,
    /// Revocation was already recorded.
    #[error("personal access token is already revoked")]
    AlreadyRevoked,
}

/// Safe token-local authentication or authorization denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PersonalAccessTokenAuthorizationError {
    /// Identifier or one-way verifier did not match.
    #[error("invalid personal access token credential")]
    InvalidCredential,
    /// The authenticated principal does not own this PAT.
    #[error("personal access token owner does not match")]
    WrongOwner,
    /// Trusted time predates the token's creation metadata.
    #[error("personal access token is not yet valid")]
    NotYetValid,
    /// Revocation is immediately effective.
    #[error("personal access token is revoked")]
    Revoked,
    /// Exclusive expiry has passed.
    #[error("personal access token is expired")]
    Expired,
    /// Requested Git operation or repository is outside token scope.
    #[error("personal access token does not permit this request")]
    OutOfScope,
}
