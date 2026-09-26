use forge_domain::{ProjectId, RepositoryId};
use identity_domain::OrganizationId;
use serde::{Deserialize, Serialize};

use super::{MAX_DESTINATIONS, SecretValueError};

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
