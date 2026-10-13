//! Immutable metadata creation correlation, independent of any orchestration domain.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Invalid creation identifiers or generation counters.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("volume creation identity requires non-nil identifiers and positive bounded counters")]
pub struct VolumeCreationIdentityError;

macro_rules! creation_identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);
        impl $name {
            /// Validates a persisted or predicted identifier.
            ///
            /// # Errors
            /// Rejects nil identifiers.
            pub const fn from_uuid(value: Uuid) -> Result<Self, VolumeCreationIdentityError> {
                if value.is_nil() {
                    Err(VolumeCreationIdentityError)
                } else {
                    Ok(Self(value))
                }
            }
            /// Returns the immutable identifier.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
        impl TryFrom<Uuid> for $name {
            type Error = VolumeCreationIdentityError;
            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                Self::from_uuid(value)
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
creation_identifier!(
    VolumeCreationOperationId,
    "Stable identity of one metadata creation operation, independent of retries."
);
creation_identifier!(
    VolumeOwnershipScopeId,
    "Opaque owner scope; possessing this label grants no authority."
);
creation_identifier!(
    VolumeCreationSealId,
    "Immutable seal allocated only when a metadata row is first created."
);

/// Untrusted original effect correlation. Validate before using it as intent.
///
/// These labels do not authorize an effect. A trusted caller must compare them
/// with its own authoritative command ledger before invoking a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeCreationIdentityWire {
    /// Immutable owner scope.
    pub scope_id: VolumeOwnershipScopeId,
    /// Stable metadata creation operation.
    pub operation_id: VolumeCreationOperationId,
    /// Original admitted command.
    pub command_id: Uuid,
    /// Original claimed effect operation.
    pub effect_id: Uuid,
    /// Original immutable attempt.
    pub attempt_id: Uuid,
    /// Original authenticated actor, never a delegation grant.
    pub actor_id: Uuid,
    /// Original effect request, distinct from registration request provenance.
    pub request_id: Uuid,
    /// Exact resource CAS version at claim admission.
    pub resource_version: u64,
    /// Exact original effect generation.
    pub generation: u64,
    /// Immutable resolved effect input fingerprint.
    pub input_hash: [u8; 32],
    /// Immutable owner intent canonical fingerprint.
    pub intent_hash: [u8; 32],
}

/// Validated correlation; serialization is evidence, never reusable authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct VolumeCreationIdentity(VolumeCreationIdentityWire);
impl VolumeCreationIdentity {
    /// Returns the immutable original correlation.
    #[must_use]
    pub const fn fields(&self) -> &VolumeCreationIdentityWire {
        &self.0
    }
}
impl TryFrom<VolumeCreationIdentityWire> for VolumeCreationIdentity {
    type Error = VolumeCreationIdentityError;
    fn try_from(value: VolumeCreationIdentityWire) -> Result<Self, Self::Error> {
        if [
            value.command_id,
            value.effect_id,
            value.attempt_id,
            value.actor_id,
            value.request_id,
        ]
        .iter()
        .any(Uuid::is_nil)
            || value.resource_version == 0
            || value.generation == 0
            || value.resource_version > i64::MAX.cast_unsigned()
            || value.generation > i64::MAX.cast_unsigned()
        {
            return Err(VolumeCreationIdentityError);
        }
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wire() -> VolumeCreationIdentityWire {
        VolumeCreationIdentityWire {
            scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).expect("scope"),
            operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).expect("operation"),
            command_id: Uuid::new_v4(),
            effect_id: Uuid::new_v4(),
            attempt_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            resource_version: 1,
            generation: 1,
            input_hash: [1; 32],
            intent_hash: [2; 32],
        }
    }
    #[test]
    fn original_effect_counters_and_identity_are_bounded() {
        let valid = wire();
        assert!(VolumeCreationIdentity::try_from(valid.clone()).is_ok());
        let mut invalid = valid.clone();
        invalid.actor_id = Uuid::nil();
        assert!(VolumeCreationIdentity::try_from(invalid).is_err());
        let mut invalid = valid.clone();
        invalid.generation = 0;
        assert!(VolumeCreationIdentity::try_from(invalid).is_err());
        let mut invalid = valid;
        invalid.resource_version = u64::MAX;
        assert!(VolumeCreationIdentity::try_from(invalid).is_err());
    }
    #[test]
    fn identifiers_cannot_reconstruct_nil() {
        assert!(VolumeCreationOperationId::from_uuid(Uuid::nil()).is_err());
        assert!(VolumeOwnershipScopeId::from_uuid(Uuid::nil()).is_err());
        assert!(VolumeCreationSealId::from_uuid(Uuid::nil()).is_err());
    }
}
