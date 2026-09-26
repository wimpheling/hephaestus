use identity_domain::OrganizationId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::SecretSlotKey;

/// Stable non-sensitive reason that secret resolution failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretDiagnosticCode {
    /// A referenced object does not exist or is deliberately hidden.
    Missing,
    /// Secret authority was revoked.
    Revoked,
    /// Time-bounded authority expired.
    Expired,
    /// Target repository or attachment is outside the import scope.
    OutOfScope,
    /// Requested raw/brokered delivery does not match the authority.
    WrongMode,
    /// Requested phase is not declared or delegated.
    WrongPhase,
    /// Authorization denied the actor or runtime.
    Unauthorized,
    /// A required declaration has no binding.
    RequiredBindingMissing,
    /// Current platform policy cannot implement the declared delivery.
    Unsupported,
}

/// Structured safe diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretDiagnostic {
    /// Stable machine-readable code.
    pub code: SecretDiagnosticCode,
    /// Symbolic slot, when safe and relevant.
    pub slot: Option<SecretSlotKey>,
    /// Non-sensitive explanation.
    pub message: String,
}

/// Deterministic command identity derived from typed non-sensitive inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretCommandKey([u8; 32]);

impl SecretCommandKey {
    /// Derives a command identity with length-prefixed fields to prevent
    /// concatenation ambiguity.
    #[must_use]
    pub fn derive(operation: &str, fields: &[&[u8]]) -> Self {
        let mut digest = Sha256::new();
        update_field(&mut digest, operation.as_bytes());
        for field in fields {
            update_field(&mut digest, field);
        }
        Self(digest.finalize().into())
    }

    /// Returns the raw SHA-256 identity.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

/// Validation failure for a secret-domain value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretValueError {
    /// A name, alias, or slot is malformed.
    #[error("{kind} must be a bounded lowercase identifier")]
    InvalidKey {
        /// Value-object kind; never the rejected input.
        kind: &'static str,
    },
    /// Plaintext size is outside the accepted bound.
    #[error("secret value must contain between 1 and 65536 bytes")]
    InvalidSecretSize,
    /// Runtime bearer credential size is outside the accepted bound.
    #[error("runtime credential must contain between 32 and 256 bytes")]
    InvalidCredentialSize,
    /// A use policy has no mode or phase.
    #[error("secret use policy must include at least one delivery mode and phase")]
    EmptyPolicy,
    /// A destination is malformed or the list is too large.
    #[error("secret destination policy is invalid")]
    InvalidDestination,
    /// Source and target organizations differ.
    #[error("secret delegation cannot cross organization boundaries")]
    CrossOrganization,
    /// The requested lifecycle transition is invalid.
    #[error("invalid secret lifecycle transition")]
    InvalidTransition,
}

/// Verifies source/target tenancy before creating a grant.
///
/// # Errors
///
/// Returns [`SecretValueError::CrossOrganization`] unless both objects belong
/// to the same organization.
pub fn validate_tenant_boundary(
    owner_organization_id: OrganizationId,
    target_organization_id: OrganizationId,
) -> Result<(), SecretValueError> {
    if owner_organization_id.as_uuid() == target_organization_id.as_uuid() {
        Ok(())
    } else {
        Err(SecretValueError::CrossOrganization)
    }
}
