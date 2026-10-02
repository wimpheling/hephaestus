use std::collections::BTreeSet;

use serde::Serialize;

use crate::{
    LEGACY_STATE_VOLUME_GUEST_PATH, LEGACY_STATE_VOLUME_SLOT, RunVolumeIdentity, VolumeAccessMode,
    VolumeContractError, VolumeMountScope, VolumeSlotDeclaration, validate_volume_slots,
};

/// Requested acquisition provenance, to be proved by canonical metadata.
///
/// Historical unmatched evidence is deliberately absent: it can be recovered,
/// but cannot be submitted as authority for a new acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeSelectionOrigin {
    /// An exact authored named revision binding and its live mount grant.
    Explicit,
    /// The frozen requires-state declaration bound in named mode, without origin fallback.
    LegacyDeclaration,
    /// Actual legacy requires-state release, same-origin volume and scalar pointer.
    LegacyOrigin,
}

/// Bounded request for one exact mount; constructing it does not authorize access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunVolumeSelection {
    identity: RunVolumeIdentity,
    scope: VolumeMountScope,
    declaration: VolumeSlotDeclaration,
    origin: VolumeSelectionOrigin,
}

impl RunVolumeSelection {
    /// Constructs a syntactically exact request for authoritative repository validation.
    ///
    /// # Errors
    ///
    /// Rejects mismatched consumer identity, slot or mode. Legacy requests must
    /// use the exact required built-in state declaration; storage additionally
    /// proves requires-state, ownership, origin and the legacy scalar pointer.
    pub fn new(
        identity: RunVolumeIdentity,
        scope: VolumeMountScope,
        declaration: VolumeSlotDeclaration,
        origin: VolumeSelectionOrigin,
    ) -> Result<Self, VolumeContractError> {
        if !identity.matches_scope(&scope)
            || scope.slot() != declaration.slot()
            || scope.access_mode() != declaration.access_mode()
            || (origin != VolumeSelectionOrigin::Explicit
                && (declaration.slot().as_str() != LEGACY_STATE_VOLUME_SLOT
                    || declaration.guest_path().as_str() != LEGACY_STATE_VOLUME_GUEST_PATH
                    || declaration.access_mode() != VolumeAccessMode::ReadWrite
                    || !declaration.required()
                    || declaration.minimum_capacity_bytes() != 1))
        {
            return Err(VolumeContractError::InvalidRunSelection);
        }
        Ok(Self {
            identity,
            scope,
            declaration,
            origin,
        })
    }

    /// Exact immutable run consumer identity.
    #[must_use]
    pub const fn identity(&self) -> RunVolumeIdentity {
        self.identity
    }
    /// Exact resource, slot, mode and frozen release contract hash.
    #[must_use]
    pub const fn scope(&self) -> &VolumeMountScope {
        &self.scope
    }
    /// Frozen guest path, mode and capacity declaration.
    #[must_use]
    pub const fn declaration(&self) -> &VolumeSlotDeclaration {
        &self.declaration
    }
    /// Requested provenance that metadata must prove before acquisition.
    #[must_use]
    pub const fn origin(&self) -> VolumeSelectionOrigin {
        self.origin
    }
}

/// Zero to thirty-two distinct exact selections, sorted by slot for acquisition.
///
/// This set checks shape only. Full release completeness, live authority and
/// permanent consumer closure must be checked before acquiring any member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunVolumeSelections {
    identity: RunVolumeIdentity,
    selections: Vec<RunVolumeSelection>,
}

impl RunVolumeSelections {
    /// Validates a complete request set without silently discarding duplicates.
    ///
    /// # Errors
    ///
    /// Rejects excessive counts, mismatched run identities, duplicate slots,
    /// reused resources, overlapping paths and mixed legacy/named provenance.
    pub fn new(
        identity: RunVolumeIdentity,
        mut selections: Vec<RunVolumeSelection>,
    ) -> Result<Self, VolumeContractError> {
        let declarations = selections
            .iter()
            .map(|item| item.declaration.clone())
            .collect::<Vec<_>>();
        validate_volume_slots(&declarations)?;
        let mut volumes = BTreeSet::new();
        let contract_hash = selections
            .first()
            .map(|item| item.scope.release_contract_hash());
        for selection in &selections {
            if selection.identity != identity
                || Some(selection.scope.release_contract_hash()) != contract_hash
                || (selection.origin == VolumeSelectionOrigin::LegacyOrigin
                    && selections.len() != 1)
            {
                return Err(VolumeContractError::InvalidRunSelection);
            }
            if !volumes.insert(selection.scope.volume_id()) {
                return Err(VolumeContractError::ReusedVolume(
                    selection.scope.volume_id(),
                ));
            }
        }
        selections.sort_by(|left, right| left.scope.slot().cmp(right.scope.slot()));
        Ok(Self {
            identity,
            selections,
        })
    }

    /// Exact immutable consumer identity, including for an empty set.
    #[must_use]
    pub const fn identity(&self) -> RunVolumeIdentity {
        self.identity
    }
    /// Canonical acquisition and compensation order.
    #[must_use]
    pub fn selections(&self) -> &[RunVolumeSelection] {
        &self.selections
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
