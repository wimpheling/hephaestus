#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
/// Registry token parsing, issuance, or verification failure.
pub enum RegistryTokenError {
    /// The registry service was not canonical.
    #[error("registry service is invalid")]
    InvalidService,
    /// The repository path was not canonical.
    #[error("repository name is invalid")]
    InvalidRepository,
    /// The scope did not use the exact repository grammar.
    #[error("registry scope is invalid")]
    InvalidScope,
    /// A scope included an unsupported or repeated action.
    #[error("registry scope actions are invalid")]
    InvalidActions,
    /// Multiple scopes tried to describe one repository.
    #[error("registry request repeats a repository scope")]
    DuplicateRepositoryScope,
    /// An issuer or subject contained invalid claim text.
    #[error("registry token claim text is invalid")]
    InvalidClaimText,
    /// A key identifier was invalid.
    #[error("registry key identifier is invalid")]
    InvalidKeyId,
    /// Token lifetime was zero or longer than the allowed maximum.
    #[error("registry token lifetime is invalid")]
    InvalidLifetime,
    /// HMAC key material was too short.
    #[error("registry signing material is insufficient")]
    InsufficientKeyMaterial,
    /// Asymmetric signing or verification key material could not be parsed.
    #[error("registry key material is invalid")]
    InvalidKeyMaterial(#[source] jsonwebtoken::errors::Error),
    /// The requested service did not match this issuer's configured audience.
    #[error("registry requested service does not match")]
    ServiceMismatch,
    /// Adding a lifetime to the supplied timestamp overflowed.
    #[error("registry token timestamp overflowed")]
    TimestampOverflow,
    /// JWT encoding failed.
    #[error("registry token encoding failed")]
    Encode(#[source] jsonwebtoken::errors::Error),
    /// JWT header or signature decoding failed.
    #[error("registry token decoding failed")]
    Decode(#[source] jsonwebtoken::errors::Error),
    /// The token did not carry a key identifier.
    #[error("registry token key identifier is missing")]
    MissingKeyId,
    /// No configured verifier key matched the token key identifier.
    #[error("registry token key identifier is unknown")]
    UnknownKeyId,
    /// The token algorithm did not match the selected verification key.
    #[error("registry token algorithm is invalid")]
    UnexpectedAlgorithm,
    /// A verifier key set contained no keys.
    #[error("registry verifier has no keys")]
    NoVerificationKeys,
    /// A verifier key set repeated a key identifier.
    #[error("registry verifier repeats a key identifier")]
    DuplicateKeyId,
    /// The token issuer did not match exactly.
    #[error("registry token issuer does not match")]
    IssuerMismatch,
    /// The token audience did not match exactly.
    #[error("registry token audience does not match")]
    AudienceMismatch,
    /// The token has expired.
    #[error("registry token has expired")]
    Expired,
    /// The token is not yet valid.
    #[error("registry token is not yet valid")]
    NotYetValid,
    /// The token issued-at timestamp is in the future.
    #[error("registry token issued-at timestamp is in the future")]
    IssuedInFuture,
    /// The token's time bounds are internally inconsistent.
    #[error("registry token time bounds are invalid")]
    InvalidTimeBounds,
    /// The token lifetime exceeds this verifier's policy.
    #[error("registry token lifetime exceeds verifier policy")]
    LifetimeExceeded,
    /// The signed access entries were not canonical.
    #[error("registry token access claims are invalid")]
    InvalidAccessClaims,
}
