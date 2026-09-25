use crate::{ReleaseCommandKey, ReleaseId, UiInstallationGenerationId, UiInstallationId};
use forge_domain::OrganizationId;
use serde::{Deserialize, Serialize};

use super::UiInstallationTarget;
use crate::ui::UiKey;

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

    /// Digests an install request with an optional externally asserted tenant.
    ///
    /// The `None` form delegates to the stable v1 digest so compatibility
    /// wrappers and existing receipts retain their canonical identity. The
    /// `Some` form uses a new domain separator and binds the expected tenant.
    #[must_use]
    pub fn install_with_expected_organization(
        target: UiInstallationTarget,
        release_id: ReleaseId,
        ui_key: &UiKey,
        expected_organization: Option<OrganizationId>,
    ) -> Self {
        Self::install_with_expected_organization_and_git_ack(
            target,
            release_id,
            ui_key,
            expected_organization,
            false,
        )
    }

    /// Digests an install request including its explicit Git authority approval.
    #[must_use]
    pub fn install_with_expected_organization_and_git_ack(
        target: UiInstallationTarget,
        release_id: ReleaseId,
        ui_key: &UiKey,
        expected_organization: Option<OrganizationId>,
        acknowledge_repository_git_access: bool,
    ) -> Self {
        if !acknowledge_repository_git_access {
            return expected_organization.map_or_else(
                || Self::install(target, release_id, ui_key),
                |organization| {
                    Self::derive(
                        "ui-installation-input-v2/install",
                        &[
                            target.scope_name().as_bytes(),
                            target.as_uuid().as_bytes(),
                            organization.as_uuid().as_bytes(),
                            release_id.as_uuid().as_bytes(),
                            ui_key.as_str().as_bytes(),
                        ],
                    )
                },
            );
        }
        expected_organization.map_or_else(
            || {
                Self::derive(
                    "ui-installation-input-v2/install",
                    &[
                        target.scope_name().as_bytes(),
                        target.as_uuid().as_bytes(),
                        release_id.as_uuid().as_bytes(),
                        ui_key.as_str().as_bytes(),
                        &[u8::from(acknowledge_repository_git_access)],
                    ],
                )
            },
            |organization| {
                Self::derive(
                    "ui-installation-input-v3/install",
                    &[
                        target.scope_name().as_bytes(),
                        target.as_uuid().as_bytes(),
                        organization.as_uuid().as_bytes(),
                        release_id.as_uuid().as_bytes(),
                        ui_key.as_str().as_bytes(),
                        &[u8::from(acknowledge_repository_git_access)],
                    ],
                )
            },
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
