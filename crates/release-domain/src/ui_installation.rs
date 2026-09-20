//! Domain primitives for project and repository UI installations.

use crate::{
    ReleaseCommandKey, ReleaseId, ReleaseValueError, UiInstallationGenerationId, UiInstallationId,
    ui::UiKey,
};
use forge_domain::{ProjectId, RepositoryId};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Maximum UTF-8 byte length of a caller-supplied idempotency key.
pub const MAX_UI_INSTALLATION_CALLER_KEY_BYTES: usize = 256;

/// A bounded opaque caller idempotency key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct UiInstallationCallerKey(String);

impl UiInstallationCallerKey {
    /// Parses a bounded caller key while preserving its bytes.
    ///
    /// # Errors
    ///
    /// Returns an invalid-key error for empty, NUL-containing, or oversized
    /// input. Other whitespace and control bytes remain caller data.
    pub fn parse(value: impl Into<String>) -> Result<Self, ReleaseValueError> {
        let value = value.into();
        let valid = (1..=MAX_UI_INSTALLATION_CALLER_KEY_BYTES).contains(&value.len())
            && !value.as_bytes().contains(&0);
        if valid {
            Ok(Self(value))
        } else {
            Err(ReleaseValueError::InvalidKey {
                kind: "ui installation caller idempotency key",
            })
        }
    }

    /// Returns the validated caller key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UiInstallationCallerKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for UiInstallationCallerKey {
    type Error = ReleaseValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<UiInstallationCallerKey> for String {
    fn from(value: UiInstallationCallerKey) -> Self {
        value.0
    }
}

/// Navigation owner for an installation. There is intentionally no global
/// variant until global ownership is specified by product policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", content = "id", rename_all = "snake_case")]
pub enum UiInstallationTarget {
    /// A project-wide installation.
    Project(ProjectId),
    /// An installation scoped to one repository.
    Repository(RepositoryId),
}

impl UiInstallationTarget {
    /// Creates a project-scoped target.
    #[must_use]
    pub const fn project(project_id: ProjectId) -> Self {
        Self::Project(project_id)
    }

    /// Creates a repository-scoped target.
    #[must_use]
    pub const fn repository(repository_id: RepositoryId) -> Self {
        Self::Repository(repository_id)
    }

    /// Returns the target UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        match self {
            Self::Project(id) => id.as_uuid(),
            Self::Repository(id) => id.as_uuid(),
        }
    }

    /// Returns the stable scope discriminator used by canonical hashing.
    #[must_use]
    pub const fn scope_name(self) -> &'static str {
        match self {
            Self::Project(_) => "project",
            Self::Repository(_) => "repository",
        }
    }
}

/// Lifecycle of a stable installation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiInstallationState {
    /// The current generation may be projected and served.
    Enabled,
    /// The installation remains retained but is unavailable for projection.
    Disabled,
    /// The installation is terminal and its key may be reused by a new identity.
    Removed,
}

impl UiInstallationState {
    /// Returns whether an installation may move to the next state.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Enabled | Self::Disabled,
                Self::Enabled | Self::Disabled | Self::Removed
            )
        )
    }
}

/// Installation command operation used for actor-bound idempotency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiInstallationOperation {
    /// Create a stable installation identity and its first generation.
    Install,
    /// Create a fresh generation or reactivate an installation.
    Activate,
    /// Roll back to a selected published release as a fresh generation.
    Rollback,
    /// Disable an installation without deleting its history.
    Disable,
    /// Terminally remove an installation.
    Remove,
}

impl UiInstallationOperation {
    /// Returns the canonical operation spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Activate => "activate",
            Self::Rollback => "rollback",
            Self::Disable => "disable",
            Self::Remove => "remove",
        }
    }
}

/// Actor, operation, and caller key that identify one replayable command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiInstallationCommandIdentity {
    actor_id: Uuid,
    operation: UiInstallationOperation,
    caller_key: UiInstallationCallerKey,
}

impl UiInstallationCommandIdentity {
    /// Creates an actor-bound command identity.
    #[must_use]
    pub const fn new(
        actor_id: Uuid,
        operation: UiInstallationOperation,
        caller_key: UiInstallationCallerKey,
    ) -> Self {
        Self {
            actor_id,
            operation,
            caller_key,
        }
    }

    /// Returns the derived actor-bound command key.
    #[must_use]
    pub fn command_key(&self) -> ReleaseCommandKey {
        ReleaseCommandKey::derive(
            "ui-installation-command-v1",
            &[
                self.operation.as_str().as_bytes(),
                self.actor_id.as_bytes(),
                self.caller_key.as_str().as_bytes(),
            ],
        )
    }
}

/// Digest of canonical installation mutation input, stored separately from
/// the actor-bound command key so changed input under one key can conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UiInstallationInputDigest([u8; 32]);

impl UiInstallationInputDigest {
    /// Digests an initial install request.
    #[must_use]
    pub fn install(target: UiInstallationTarget, release_id: ReleaseId, ui_key: &UiKey) -> Self {
        Self::derive(
            "ui-installation-input-v1/install",
            &[
                target.scope_name().as_bytes(),
                target.as_uuid().as_bytes(),
                release_id.as_uuid().as_bytes(),
                ui_key.as_str().as_bytes(),
            ],
        )
    }

    /// Digests a fresh activation or reactivation request.
    #[must_use]
    pub fn activate(
        installation_id: UiInstallationId,
        expected_generation: Option<UiInstallationGenerationId>,
        release_id: ReleaseId,
        ui_key: &UiKey,
    ) -> Self {
        expected_generation.map_or_else(
            || {
                Self::derive(
                    "ui-installation-input-v1/activate",
                    &[
                        installation_id.as_uuid().as_bytes(),
                        b"none",
                        release_id.as_uuid().as_bytes(),
                        ui_key.as_str().as_bytes(),
                    ],
                )
            },
            |generation_id| {
                Self::derive(
                    "ui-installation-input-v1/activate",
                    &[
                        installation_id.as_uuid().as_bytes(),
                        b"some",
                        generation_id.as_uuid().as_bytes(),
                        release_id.as_uuid().as_bytes(),
                        ui_key.as_str().as_bytes(),
                    ],
                )
            },
        )
    }

    /// Digests a rollback request with its compare-and-swap expectation.
    #[must_use]
    pub fn rollback(
        installation_id: UiInstallationId,
        expected_generation: Option<UiInstallationGenerationId>,
        release_id: ReleaseId,
        ui_key: &UiKey,
    ) -> Self {
        expected_generation.map_or_else(
            || {
                Self::derive(
                    "ui-installation-input-v1/rollback",
                    &[
                        installation_id.as_uuid().as_bytes(),
                        b"none",
                        release_id.as_uuid().as_bytes(),
                        ui_key.as_str().as_bytes(),
                    ],
                )
            },
            |generation_id| {
                Self::derive(
                    "ui-installation-input-v1/rollback",
                    &[
                        installation_id.as_uuid().as_bytes(),
                        b"some",
                        generation_id.as_uuid().as_bytes(),
                        release_id.as_uuid().as_bytes(),
                        ui_key.as_str().as_bytes(),
                    ],
                )
            },
        )
    }

    /// Digests a disable request with its compare-and-swap expectation.
    #[must_use]
    pub fn disable(
        installation_id: UiInstallationId,
        expected_generation: Option<UiInstallationGenerationId>,
    ) -> Self {
        Self::lifecycle("disable", installation_id, expected_generation)
    }

    /// Digests a remove request with its compare-and-swap expectation.
    #[must_use]
    pub fn remove(
        installation_id: UiInstallationId,
        expected_generation: Option<UiInstallationGenerationId>,
    ) -> Self {
        Self::lifecycle("remove", installation_id, expected_generation)
    }

    /// Returns raw digest bytes for persistence.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn lifecycle(
        operation: &str,
        installation_id: UiInstallationId,
        expected_generation: Option<UiInstallationGenerationId>,
    ) -> Self {
        let operation = format!("ui-installation-input-v1/{operation}");
        expected_generation.map_or_else(
            || Self::derive(&operation, &[installation_id.as_uuid().as_bytes(), b"none"]),
            |generation_id| {
                Self::derive(
                    &operation,
                    &[
                        installation_id.as_uuid().as_bytes(),
                        b"some",
                        generation_id.as_uuid().as_bytes(),
                    ],
                )
            },
        )
    }

    fn derive(operation: &str, fields: &[&[u8]]) -> Self {
        Self(*ReleaseCommandKey::derive(operation, fields).as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    fn key(value: &str) -> UiKey {
        UiKey::parse(value).expect("valid UI key")
    }

    fn caller(value: &str) -> UiInstallationCallerKey {
        UiInstallationCallerKey::parse(value).expect("valid caller key")
    }

    #[test]
    fn caller_key_is_bounded_and_printable() {
        assert!(UiInstallationCallerKey::parse("request-1").is_ok());
        assert!(UiInstallationCallerKey::parse("").is_err());
        assert!(UiInstallationCallerKey::parse(" request").is_ok());
        assert!(UiInstallationCallerKey::parse("request\n1").is_ok());
        assert!(UiInstallationCallerKey::parse("request\0").is_err());
        assert!(
            UiInstallationCallerKey::parse("a".repeat(MAX_UI_INSTALLATION_CALLER_KEY_BYTES))
                .is_ok()
        );
        assert!(
            UiInstallationCallerKey::parse("a".repeat(MAX_UI_INSTALLATION_CALLER_KEY_BYTES + 1))
                .is_err()
        );
    }

    #[test]
    fn command_key_binds_actor_operation_and_caller_key() {
        let actor = Uuid::from_u128(1);
        let first = UiInstallationCommandIdentity::new(
            actor,
            UiInstallationOperation::Install,
            caller("request-1"),
        );
        assert_eq!(
            first.command_key(),
            UiInstallationCommandIdentity::new(
                actor,
                UiInstallationOperation::Install,
                caller("request-1"),
            )
            .command_key()
        );
        assert_ne!(
            first.command_key(),
            UiInstallationCommandIdentity::new(
                Uuid::from_u128(2),
                UiInstallationOperation::Install,
                caller("request-1"),
            )
            .command_key()
        );
        assert_ne!(
            first.command_key(),
            UiInstallationCommandIdentity::new(
                actor,
                UiInstallationOperation::Activate,
                caller("request-1"),
            )
            .command_key()
        );
        assert_ne!(
            first.command_key(),
            UiInstallationCommandIdentity::new(
                actor,
                UiInstallationOperation::Install,
                caller("request-2"),
            )
            .command_key()
        );
    }

    #[test]
    fn changed_input_keeps_command_key_but_changes_input_digest() {
        let actor = Uuid::from_u128(1);
        let identity = UiInstallationCommandIdentity::new(
            actor,
            UiInstallationOperation::Install,
            caller("request-1"),
        );
        let same_identity = UiInstallationCommandIdentity::new(
            actor,
            UiInstallationOperation::Install,
            caller("request-1"),
        );
        let target = UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(2)));
        let release = ReleaseId::from_uuid(Uuid::from_u128(3));
        let first = UiInstallationInputDigest::install(target, release, &key("assistant"));
        let changed = UiInstallationInputDigest::install(target, release, &key("other"));
        assert_eq!(identity.command_key(), same_identity.command_key());
        assert_ne!(first, changed);
    }

    #[test]
    fn compare_and_swap_and_operation_inputs_are_distinct() {
        let installation = UiInstallationId::from_uuid(Uuid::from_u128(1));
        let generation = UiInstallationGenerationId::from_uuid(Uuid::from_u128(2));
        let release = ReleaseId::from_uuid(Uuid::from_u128(3));
        let ui_key = key("assistant");
        assert_ne!(
            UiInstallationInputDigest::activate(installation, None, release, &ui_key),
            UiInstallationInputDigest::activate(installation, Some(generation), release, &ui_key,)
        );
        assert_ne!(
            UiInstallationInputDigest::disable(installation, Some(generation)),
            UiInstallationInputDigest::remove(installation, Some(generation)),
        );
        assert_ne!(
            UiInstallationInputDigest::activate(installation, Some(generation), release, &ui_key),
            UiInstallationInputDigest::rollback(installation, Some(generation), release, &ui_key),
        );
        assert_ne!(
            UiInstallationInputDigest::install(
                UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(4))),
                release,
                &ui_key,
            ),
            UiInstallationInputDigest::install(
                UiInstallationTarget::repository(RepositoryId::from_uuid(Uuid::from_u128(4))),
                release,
                &ui_key,
            ),
        );
    }

    #[test]
    fn fresh_caller_key_reuses_input_digest_but_changes_command_key() {
        let first = UiInstallationCommandIdentity::new(
            Uuid::from_u128(1),
            UiInstallationOperation::Activate,
            caller("request-1"),
        );
        let second = UiInstallationCommandIdentity::new(
            Uuid::from_u128(1),
            UiInstallationOperation::Activate,
            caller("request-2"),
        );
        let input = UiInstallationInputDigest::activate(
            UiInstallationId::from_uuid(Uuid::from_u128(2)),
            None,
            ReleaseId::from_uuid(Uuid::from_u128(3)),
            &key("assistant"),
        );
        let repeated = UiInstallationInputDigest::activate(
            UiInstallationId::from_uuid(Uuid::from_u128(2)),
            None,
            ReleaseId::from_uuid(Uuid::from_u128(3)),
            &key("assistant"),
        );
        assert_eq!(input, repeated);
        assert_ne!(first.command_key(), second.command_key());
    }

    #[test]
    fn canonical_v1_hash_vectors_are_stable() {
        let command = UiInstallationCommandIdentity::new(
            Uuid::from_u128(1),
            UiInstallationOperation::Install,
            caller("request-1"),
        )
        .command_key();
        assert_eq!(
            hex(command.as_bytes()),
            "f8c3340ccd9631ebd1f46fb3ef028baf4867df97057572326af43bf478f81a2c"
        );
        let digest = UiInstallationInputDigest::install(
            UiInstallationTarget::project(ProjectId::from_uuid(Uuid::from_u128(2))),
            ReleaseId::from_uuid(Uuid::from_u128(3)),
            &key("assistant"),
        );
        assert_eq!(
            hex(digest.as_bytes()),
            "7fa65774e07cca5430f05b76b3f47307bc6112d3b042c2368fe8d6dc7dc6b536"
        );
    }

    fn hex(value: &[u8; 32]) -> String {
        let mut output = String::with_capacity(64);
        for byte in value {
            write!(&mut output, "{byte:02x}").expect("write into string");
        }
        output
    }

    #[test]
    fn lifecycle_keeps_removed_terminal() {
        assert!(UiInstallationState::Enabled.can_transition_to(UiInstallationState::Enabled));
        assert!(UiInstallationState::Enabled.can_transition_to(UiInstallationState::Disabled));
        assert!(UiInstallationState::Disabled.can_transition_to(UiInstallationState::Disabled));
        assert!(UiInstallationState::Disabled.can_transition_to(UiInstallationState::Enabled));
        assert!(UiInstallationState::Disabled.can_transition_to(UiInstallationState::Removed));
        for state in [
            UiInstallationState::Enabled,
            UiInstallationState::Disabled,
            UiInstallationState::Removed,
        ] {
            assert!(!UiInstallationState::Removed.can_transition_to(state));
        }
    }
}
