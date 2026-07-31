//! Provider-neutral secret ownership, delegation, binding, and lease contracts.
//!
//! This crate deliberately has no plaintext retrieval contract. [`SecretValue`]
//! can enter a storage implementation, but its formatting and serialization
//! representations are always redacted.

use forge_domain::{ProjectId, RepositoryId};
use identity_domain::OrganizationId;
use runtime_types::RunId;
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
use time::OffsetDateTime;
use uuid::Uuid;

/// Maximum accepted plaintext size for one secret version.
pub const MAX_SECRET_VALUE_BYTES: usize = 65_536;
/// Maximum number of destination constraints on a grant or binding.
pub const MAX_DESTINATIONS: usize = 32;

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

            /// Creates an identifier from its UUID representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID representation.
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

identifier!(SecretId, "A stable identifier for an owned secret.");
identifier!(
    SecretVersionId,
    "A stable identifier for an immutable encrypted secret version."
);
identifier!(
    SecretGrantId,
    "A stable identifier for an explicit source-side delegation grant."
);
identifier!(
    SecretImportId,
    "A stable identifier for an accepted opaque secret import."
);
identifier!(
    AgentSecretBindingId,
    "A stable identifier for an immutable agent-revision secret binding."
);
identifier!(
    SecretLeaseId,
    "A stable identifier for short-lived runtime secret authority."
);
identifier!(
    SecretRuntimeSessionId,
    "A stable identifier for one authenticated runtime secret session."
);

macro_rules! bounded_key {
    ($name:ident, $description:literal, $minimum:expr, $maximum:expr) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Parses and validates a key.
            ///
            /// # Errors
            ///
            /// Returns [`SecretValueError`] if the value is not a bounded,
            /// lowercase identifier.
            pub fn parse(value: impl Into<String>) -> Result<Self, SecretValueError> {
                let value = value.into();
                let valid = ($minimum..=$maximum).contains(&value.len())
                    && value.bytes().enumerate().all(|(index, byte)| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || ((byte == b'_' || byte == b'-') && index > 0)
                    });
                if !valid {
                    return Err(SecretValueError::InvalidKey {
                        kind: stringify!($name),
                    });
                }
                Ok(Self(value))
            }

            /// Returns the validated key.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl TryFrom<String> for $name {
            type Error = SecretValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_key!(SecretName, "A name unique within one secret owner.", 1, 128);
bounded_key!(
    SecretAlias,
    "A target-local name for an opaque secret import.",
    1,
    128
);
bounded_key!(
    SecretSlotKey,
    "A symbolic secret capability declared by a release.",
    1,
    64
);

/// A plaintext value that is impossible to expose through formatting or
/// serialization.
///
/// Callers should keep this value short-lived and pass it directly to an
/// encrypted store or ephemeral runtime materializer.
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    /// Accepts bounded non-empty plaintext.
    ///
    /// # Errors
    ///
    /// Returns [`SecretValueError::InvalidSecretSize`] for empty or oversized
    /// input.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, SecretValueError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_SECRET_VALUE_BYTES {
            return Err(SecretValueError::InvalidSecretSize);
        }
        Ok(Self(value))
    }

    /// Exposes the plaintext only to the narrow storage or delivery boundary.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Returns the non-sensitive plaintext length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the value is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        // A volatile write makes best effort to keep the wipe from being
        // optimized away. Stronger locked-memory handling belongs at the
        // concrete key-provider boundary.
        for byte in &mut self.0 {
            // SAFETY is intentionally avoided because this workspace denies
            // unsafe code; black_box retains the observable writes.
            *std::hint::black_box(byte) = 0;
        }
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Serialize for SecretValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

/// A bounded opaque runtime credential. Its stored representation is a hash,
/// never this bearer value.
pub struct OpaqueRuntimeCredential(Vec<u8>);

impl OpaqueRuntimeCredential {
    /// Accepts a credential with at least 32 bytes of entropy and at most 256
    /// bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SecretValueError::InvalidCredentialSize`] when out of bounds.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, SecretValueError> {
        let value = value.into();
        if !(32..=256).contains(&value.len()) {
            return Err(SecretValueError::InvalidCredentialSize);
        }
        Ok(Self(value))
    }

    /// Returns a one-way SHA-256 representation suitable for durable storage.
    #[must_use]
    pub fn storage_hash(&self) -> [u8; 32] {
        Sha256::digest(&self.0).into()
    }

    /// Exposes the bearer credential at the transport authentication boundary.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for OpaqueRuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueRuntimeCredential([REDACTED])")
    }
}

impl fmt::Display for OpaqueRuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Exactly one tenant owner for a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum SecretOwner {
    /// Organization-owned secret.
    Organization(OrganizationId),
    /// Project-owned secret.
    Project(ProjectId),
}

/// An exact target to which use may be delegated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum SecretTarget {
    /// Project import target.
    Project(ProjectId),
    /// Repository import target.
    Repository(RepositoryId),
}

/// Runtime delivery authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// The guest receives plaintext through an ephemeral read-only file.
    Raw,
    /// The guest receives only an opaque broker capability.
    Brokered,
}

/// Agent execution phase in which a secret may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    /// Ordinary attached-repository run.
    Normal,
    /// Candidate-release update hook.
    Update,
}

/// Lifecycle state of an owned secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStatus {
    /// Available for new resolution.
    Active,
    /// Temporarily disabled for new resolution.
    Disabled,
    /// Permanently revoked.
    Revoked,
    /// Tombstoned while encrypted material awaits purge.
    Tombstoned,
    /// All usable encrypted material has been purged.
    Purged,
}

impl SecretStatus {
    /// Returns whether the requested lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Active,
                Self::Disabled | Self::Revoked | Self::Tombstoned
            ) | (
                Self::Disabled,
                Self::Active | Self::Revoked | Self::Tombstoned
            ) | (Self::Revoked, Self::Tombstoned)
                | (Self::Tombstoned, Self::Purged)
        )
    }
}

/// Lifecycle state shared by grants, imports, bindings, and leases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityStatus {
    /// Authority is usable.
    Active,
    /// Authority was revoked and cannot be restored.
    Revoked,
    /// Authority expired.
    Expired,
}

/// Normalized bounded secret use policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretUsePolicy {
    /// Accepted delivery modes.
    pub delivery_modes: Vec<DeliveryMode>,
    /// Accepted phases.
    pub phases: Vec<ExecutionPhase>,
    /// Optional exact destinations for broker calls.
    pub destinations: Vec<String>,
}

impl SecretUsePolicy {
    /// Validates, sorts, and deduplicates a policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecretValueError`] for empty authority, invalid destination
    /// syntax, or excess destinations.
    pub fn normalized(mut self) -> Result<Self, SecretValueError> {
        self.delivery_modes.sort_unstable_by_key(|mode| *mode as u8);
        self.delivery_modes.dedup();
        self.phases.sort_unstable_by_key(|phase| *phase as u8);
        self.phases.dedup();
        self.destinations.sort_unstable();
        self.destinations.dedup();
        if self.delivery_modes.is_empty() || self.phases.is_empty() {
            return Err(SecretValueError::EmptyPolicy);
        }
        if self.destinations.len() > MAX_DESTINATIONS
            || self
                .destinations
                .iter()
                .any(|value| !valid_destination(value))
        {
            return Err(SecretValueError::InvalidDestination);
        }
        Ok(self)
    }

    /// Returns whether this policy includes an exact request.
    #[must_use]
    pub fn permits(
        &self,
        mode: DeliveryMode,
        phase: ExecutionPhase,
        destination: Option<&str>,
    ) -> bool {
        self.delivery_modes.contains(&mode)
            && self.phases.contains(&phase)
            && (self.destinations.is_empty()
                || destination
                    .is_some_and(|value| self.destinations.iter().any(|allowed| allowed == value)))
    }
}

fn valid_destination(value: &str) -> bool {
    (1..=253).contains(&value.len())
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
}

/// Durable metadata for an owned secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    /// Stable secret identifier.
    pub id: SecretId,
    /// Exact organization or project owner.
    pub owner: SecretOwner,
    /// Owner-scoped name.
    pub name: SecretName,
    /// Current lifecycle state.
    pub status: SecretStatus,
    /// Active immutable version.
    pub active_version_id: Option<SecretVersionId>,
    /// Creation time.
    pub created_at: OffsetDateTime,
}

/// Immutable encrypted-version metadata. Ciphertext is intentionally absent
/// from the provider-neutral domain model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretVersion {
    /// Version identifier.
    pub id: SecretVersionId,
    /// Parent secret.
    pub secret_id: SecretId,
    /// Monotonic owner-local version number.
    pub sequence: u64,
    /// Versioned encryption algorithm identifier.
    pub algorithm: String,
    /// Host-side key-encryption-key reference.
    pub key_reference: String,
    /// Plaintext length used for limits and audit.
    pub content_length: u32,
    /// Creation time.
    pub created_at: OffsetDateTime,
    /// Revocation time, if explicitly revoked.
    pub revoked_at: Option<OffsetDateTime>,
    /// Purge time, if cryptographic material was destroyed.
    pub purged_at: Option<OffsetDateTime>,
}

/// Explicit source-side authority offered to one exact target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretGrant {
    /// Grant identifier.
    pub id: SecretGrantId,
    /// Source secret.
    pub secret_id: SecretId,
    /// Exact target.
    pub target: SecretTarget,
    /// Bounded authority ceiling.
    pub policy: SecretUsePolicy,
    /// Current status.
    pub status: AuthorityStatus,
    /// Optional expiration.
    pub expires_at: Option<OffsetDateTime>,
}

/// Target-side acceptance of an opaque grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretImport {
    /// Import identifier.
    pub id: SecretImportId,
    /// Source grant. An import can never refer to another import.
    pub grant_id: SecretGrantId,
    /// Exact accepted target.
    pub target: SecretTarget,
    /// Target-local alias.
    pub alias: SecretAlias,
    /// Current status.
    pub status: AuthorityStatus,
}

/// Exact immutable agent-revision binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSecretBinding {
    /// Binding identifier.
    pub id: AgentSecretBindingId,
    /// Opaque accepted import.
    pub import_id: SecretImportId,
    /// Exact immutable agent-instance revision UUID.
    pub agent_instance_revision_id: Uuid,
    /// Symbolic release slot.
    pub slot: SecretSlotKey,
    /// Selected delivery authority.
    pub mode: DeliveryMode,
    /// Selected phases.
    pub phases: Vec<ExecutionPhase>,
    /// Exact selected attachments.
    pub attachment_ids: Vec<Uuid>,
    /// Normalized destination ceiling.
    pub destinations: Vec<String>,
    /// Current status.
    pub status: AuthorityStatus,
}

/// Short-lived authority bound to an exact run and secret version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLease {
    /// Lease identifier.
    pub id: SecretLeaseId,
    /// Exact runtime run.
    pub run_id: RunId,
    /// Exact binding.
    pub binding_id: AgentSecretBindingId,
    /// Exact immutable version.
    pub secret_version_id: SecretVersionId,
    /// Authorized mode.
    pub mode: DeliveryMode,
    /// Credential hash; the bearer token is never stored.
    pub runtime_credential_hash: [u8; 32],
    /// Expiration.
    pub expires_at: OffsetDateTime,
    /// Current status.
    pub status: AuthorityStatus,
    /// Whether raw material may already have been observed by the guest.
    pub raw_material_observed: bool,
}

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

#[cfg(test)]
mod tests {
    use super::{
        AuthorityStatus, DeliveryMode, ExecutionPhase, OpaqueRuntimeCredential, SecretCommandKey,
        SecretDiagnostic, SecretDiagnosticCode, SecretName, SecretSlotKey, SecretStatus,
        SecretUsePolicy, SecretValue, validate_tenant_boundary,
    };
    use identity_domain::OrganizationId;

    const SENTINEL: &str = "sentinel-do-not-disclose-9e25143a";

    #[test]
    fn validates_bounded_values_and_policies() {
        assert!(SecretName::parse("github_token").is_ok());
        assert!(SecretName::parse("").is_err());
        assert!(SecretName::parse("UPPER").is_err());
        assert!(SecretSlotKey::parse("model-1").is_ok());
        assert!(SecretSlotKey::parse("x".repeat(65)).is_err());

        let policy = SecretUsePolicy {
            delivery_modes: vec![DeliveryMode::Brokered, DeliveryMode::Brokered],
            phases: vec![ExecutionPhase::Normal],
            destinations: vec![String::from("api.example.test")],
        }
        .normalized()
        .expect("policy should validate");
        assert!(policy.permits(
            DeliveryMode::Brokered,
            ExecutionPhase::Normal,
            Some("api.example.test")
        ));
        assert!(!policy.permits(
            DeliveryMode::Raw,
            ExecutionPhase::Normal,
            Some("api.example.test")
        ));
    }

    #[test]
    fn redacts_every_plaintext_representation() {
        let value = SecretValue::new(SENTINEL.as_bytes()).expect("sentinel should be valid");
        let credential =
            OpaqueRuntimeCredential::new(SENTINEL.repeat(2)).expect("credential should be valid");
        let representations = [
            format!("{value:?}"),
            format!("{value}"),
            serde_json::to_string(&value).expect("redacted serialization should work"),
            format!("{credential:?}"),
            format!("{credential}"),
        ];
        for representation in representations {
            assert!(!representation.contains(SENTINEL));
        }
        assert_eq!(value.len(), SENTINEL.len());
        assert_ne!(credential.storage_hash(), [0; 32]);
    }

    #[test]
    fn errors_and_diagnostics_never_include_rejected_values() {
        let error = SecretName::parse(SENTINEL.to_uppercase()).expect_err("uppercase must fail");
        let diagnostic = SecretDiagnostic {
            code: SecretDiagnosticCode::Unauthorized,
            slot: Some(SecretSlotKey::parse("model").expect("slot should validate")),
            message: String::from("runtime is not authorized for this slot"),
        };
        for representation in [
            format!("{error:?}"),
            error.to_string(),
            format!("{diagnostic:?}"),
            serde_json::to_string(&diagnostic).expect("diagnostic should serialize"),
        ] {
            assert!(!representation.contains(SENTINEL));
        }
    }

    #[test]
    fn lifecycle_and_tenant_boundaries_fail_closed() {
        assert!(SecretStatus::Active.can_transition_to(SecretStatus::Disabled));
        assert!(SecretStatus::Disabled.can_transition_to(SecretStatus::Active));
        assert!(!SecretStatus::Purged.can_transition_to(SecretStatus::Active));
        assert_ne!(AuthorityStatus::Active, AuthorityStatus::Revoked);

        let organization = OrganizationId::new();
        assert!(validate_tenant_boundary(organization, organization).is_ok());
        assert!(validate_tenant_boundary(organization, OrganizationId::new()).is_err());
    }

    #[test]
    fn deterministic_command_keys_are_typed_and_unambiguous() {
        let first = SecretCommandKey::derive("rotate", &[b"ab", b"c"]);
        let repeated = SecretCommandKey::derive("rotate", &[b"ab", b"c"]);
        let ambiguous_without_lengths = SecretCommandKey::derive("rotate", &[b"a", b"bc"]);
        let other_operation = SecretCommandKey::derive("create", &[b"ab", b"c"]);
        assert_eq!(first, repeated);
        assert_ne!(first, ambiguous_without_lengths);
        assert_ne!(first, other_operation);
    }

    #[test]
    fn identifiers_parse_and_serialize() {
        let id = super::SecretId::new();
        let text = id.to_string();
        assert_eq!(
            text.parse::<super::SecretId>().expect("UUID should parse"),
            id
        );
        assert_eq!(
            serde_json::from_str::<super::SecretId>(
                &serde_json::to_string(&id).expect("identifier should serialize")
            )
            .expect("identifier should deserialize"),
            id
        );
        assert!("not-a-uuid".parse::<super::SecretId>().is_err());
    }
}
