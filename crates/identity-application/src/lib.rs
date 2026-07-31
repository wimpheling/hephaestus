//! Provider-neutral identity application operations and persistence ports.

use async_trait::async_trait;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::{Value, json};
use std::{error::Error, sync::Arc};

/// Verified external identity accepted by an internal identity mapper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedExternalIdentity {
    /// Trusted identity-provider issuer.
    pub issuer: String,
    /// Stable issuer-local subject.
    pub subject: String,
    /// Complete claims validated by the external protocol adapter.
    pub claims: Value,
}

/// Inputs captured from a verified bootstrap assertion.
pub struct ResolveIdentity {
    /// Correlation identifier for this transport request.
    pub request_id: RequestId,
    /// Domain-separated digest of the caller's idempotency key.
    pub idempotency_seed: [u8; 32],
    /// Trusted identity-provider issuer.
    pub issuer: String,
    /// Stable issuer-local subject.
    pub subject: String,
    /// Display name included in the verified assertion.
    pub display_name: String,
    /// Email address included in the verified assertion.
    pub email: String,
    /// Whether the identity provider verified the email address.
    pub email_verified: bool,
}

/// Persistence input for one idempotent verified-identity resolution.
pub struct ResolveVerifiedIdentity {
    /// Correlation identifier for the first attempted mutation.
    pub request_id: RequestId,
    /// Domain-separated digest bound to the mapped actor by the provider.
    pub idempotency_seed: [u8; 32],
    /// Verified external identity and exact claims to persist.
    pub verified: VerifiedExternalIdentity,
}

/// Canonical identity returned to an inbound transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedIdentity {
    /// Stable internal user identifier.
    pub user_id: UserId,
    /// Canonical display name stored for the internal user.
    pub display_name: String,
    /// Actor-bound logical mutation identifier.
    pub idempotency_id: RequestId,
}

/// Trusted fixture or operator input for creating an external mapping.
pub struct BootstrapIdentity {
    /// Stable internal user identifier.
    pub user_id: UserId,
    /// Initial canonical display name.
    pub display_name: String,
    /// Trusted identity-provider issuer.
    pub issuer: String,
    /// Stable issuer-local subject.
    pub subject: String,
    /// Non-authoritative provider metadata retained with the mapping.
    pub provider_metadata: Value,
}

/// Provider-neutral verified-identity mapping failures.
#[derive(Debug, thiserror::Error)]
pub enum IdentityMappingError {
    /// No internal user mapping exists for the exact issuer and subject.
    #[error("external identity is not mapped")]
    Unmapped,
    /// The mapped internal user cannot authenticate.
    #[error("mapped identity is inactive")]
    Inactive,
    /// The configured identity provider failed.
    #[error("identity mapping provider failed")]
    Provider(#[source] Box<dyn Error + Send + Sync>),
}

impl IdentityMappingError {
    /// Wraps a provider-specific failure without exposing its concrete type.
    #[must_use]
    pub fn provider(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}

/// Typed failures from idempotent identity resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveIdentityError {
    /// The verified identity cannot use the application.
    #[error("identity is not permitted")]
    PermissionDenied,
    /// The idempotency key was already used with different identity claims.
    #[error("identity resolution conflicts with an earlier retry")]
    IdempotencyConflict,
    /// The configured resolution provider failed.
    #[error("identity resolution provider failed")]
    Provider(#[source] Box<dyn Error + Send + Sync>),
}

impl ResolveIdentityError {
    /// Wraps a provider-specific failure without exposing its concrete type.
    #[must_use]
    pub fn provider(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}

/// Provider-neutral trusted identity-bootstrap failures.
#[derive(Debug, thiserror::Error)]
pub enum BootstrapIdentityError {
    /// The issuer and subject are already bound to another internal user.
    #[error("external identity mapping conflicts with an existing user")]
    Conflict,
    /// The configured bootstrap provider failed.
    #[error("identity bootstrap provider failed")]
    Provider(#[source] Box<dyn Error + Send + Sync>),
}

impl BootstrapIdentityError {
    /// Wraps a provider-specific failure without exposing its concrete type.
    #[must_use]
    pub fn provider(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}

/// Maps one protocol-verified external identity to an active internal user.
#[async_trait]
pub trait VerifiedIdentityMapper: Send + Sync {
    /// Maps and refreshes the verified profile in one provider transaction.
    async fn map_verified_identity(
        &self,
        verified: &VerifiedExternalIdentity,
        request_id: RequestId,
        trace_id: Option<&str>,
    ) -> Result<AuthenticatedIdentity, IdentityMappingError>;
}

/// Resolves one verified bootstrap assertion with actor-bound idempotency.
#[async_trait]
pub trait IdempotentIdentityResolver: Send + Sync {
    /// Resolves or replays the exact verified identity mutation.
    async fn resolve_verified_identity(
        &self,
        request: ResolveVerifiedIdentity,
    ) -> Result<ResolvedIdentity, ResolveIdentityError>;
}

/// Creates trusted internal-user and external-identity fixture mappings.
#[async_trait]
pub trait IdentityBootstrapper: Send + Sync {
    /// Creates both records atomically and rejects cross-user mapping reuse.
    async fn bootstrap_identity(
        &self,
        identity: BootstrapIdentity,
    ) -> Result<(), BootstrapIdentityError>;
}

/// SQL-free identity resolution application operation.
pub struct IdentityApplication {
    resolver: Arc<dyn IdempotentIdentityResolver>,
}

impl IdentityApplication {
    /// Creates an identity application around an injected persistence port.
    #[must_use]
    pub fn new(resolver: Arc<dyn IdempotentIdentityResolver>) -> Self {
        Self { resolver }
    }

    /// Resolves an authenticated bootstrap assertion idempotently.
    ///
    /// # Errors
    ///
    /// Returns a typed denial, conflict, or provider failure.
    pub async fn resolve_identity(
        &self,
        command: ResolveIdentity,
    ) -> Result<ResolvedIdentity, ResolveIdentityError> {
        self.resolver
            .resolve_verified_identity(ResolveVerifiedIdentity {
                request_id: command.request_id,
                idempotency_seed: command.idempotency_seed,
                verified: VerifiedExternalIdentity {
                    issuer: command.issuer,
                    subject: command.subject,
                    claims: json!({
                        "email": command.email,
                        "email_verified": command.email_verified,
                        "name": command.display_name,
                    }),
                },
            })
            .await
    }
}
