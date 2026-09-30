//! Secret application result and failure types.

use secret_domain::{SecretId, SecretVersionId};

use super::BrokerAdapterError;
use secret_store::SecretStoreError;

/// Identifiers returned after creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedSecret {
    /// Owned secret.
    pub secret_id: SecretId,
    /// Active immutable version.
    pub version_id: SecretVersionId,
}
/// Non-sensitive secret command failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SecretServiceError {
    /// Authorization denied the exact operation.
    #[error("secret command is not authorized")]
    AuthorizationDenied,
    /// Secret/grant/target is missing, hidden, revoked, or expired.
    #[error("secret authority is unavailable")]
    Unavailable,
    /// A target belongs to another organization.
    #[error("secret delegation cannot cross organization boundaries")]
    CrossOrganization,
    /// An accepted target differs from the source grant.
    #[error("secret grant target does not match the accepting target")]
    TargetMismatch,
    /// Delivery mode ceiling is empty or malformed.
    #[error("secret delivery modes are invalid")]
    InvalidDeliveryModes,
    /// Expected active version lost a compare-and-swap race.
    #[error("secret active version changed concurrently")]
    StaleActiveVersion,
    /// Version sequence cannot be represented.
    #[error("secret version sequence is exhausted")]
    VersionSequenceExhausted,
    /// Purge was attempted while usable runtime leases exist.
    #[error("secret encrypted material is retained by active leases")]
    ActiveLeases,
    /// Lifecycle does not permit this operation.
    #[error("secret lifecycle does not permit this operation")]
    InvalidLifecycle,
    /// One command key was reused for another operation.
    #[error("secret command idempotency identity conflicts")]
    IdempotencyConflict,
    /// Stored idempotency result is incomplete.
    #[error("secret command idempotency record is invalid")]
    CorruptIdempotencyRecord,
    /// Stored ownership violates the database invariant.
    #[error("stored secret owner is invalid")]
    InvalidStoredData,
    /// Release does not declare the selected symbolic slot.
    #[error("release does not declare the selected secret slot")]
    SlotNotDeclared,
    /// Requested mode, phase, or destination exceeds a declaration or grant.
    #[error("secret binding policy exceeds its declaration or grant")]
    BindingPolicyMismatch,
    /// Selected attachments or instance fall outside the import target.
    #[error("secret binding is outside the import target scope")]
    BindingOutOfScope,
    /// Active instance revision changed during configuration.
    #[error("agent instance revision changed concurrently")]
    StaleInstanceRevision,
    /// Runtime authority must be short-lived and expire in the future.
    #[error("secret runtime lease lifetime is invalid")]
    InvalidLeaseLifetime,
    /// Bearer authority was already issued and cannot be replayed.
    #[error("secret runtime credential was already issued")]
    CredentialAlreadyIssued,
    /// Presented runtime credential is unknown, expired, or run-mismatched.
    #[error("secret runtime authentication is denied")]
    RuntimeAuthenticationDenied,
    /// Broker request exceeds its exact semantic capability.
    #[error("secret broker request is denied")]
    BrokerRequestDenied,
    /// Sanitized broker response exceeded the configured bound.
    #[error("secret broker response is too large")]
    BrokerResponseTooLarge,
    /// Application adapter rejected or could not complete the request.
    #[error(transparent)]
    BrokerAdapter(#[from] BrokerAdapterError),
    /// JSON normalization failed.
    #[error("secret binding policy serialization failed")]
    Serialization(#[from] serde_json::Error),
    /// Authorization provider failure.
    #[error(transparent)]
    Authorization(#[from] authz_domain::AuthzError),
    /// Domain validation failure.
    #[error(transparent)]
    Domain(#[from] secret_domain::SecretValueError),
    /// Encryption/key-provider failure.
    #[error(transparent)]
    Encryption(#[from] SecretStoreError),
    /// Secret persistence provider failed.
    #[error("secret persistence failed")]
    Persistence,
}
