//! Exact typed mount authority scope, independent of release implementation.

use capability_domain::{AuthorityHash, CapabilitySlotKey};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, VolumeId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{VolumeAccessMode, VolumeContractError};

macro_rules! mount_identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "Uuid", into = "Uuid")]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a fresh immutable record identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            /// Reconstructs a non-nil predicted or persisted identifier.
            ///
            /// # Errors
            ///
            /// Rejects the nil identifier.
            pub fn from_uuid(value: Uuid) -> Result<Self, VolumeContractError> {
                Self::try_from(value)
            }
            /// Returns the durable identifier.
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
        impl TryFrom<Uuid> for $name {
            type Error = VolumeContractError;
            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                if value.is_nil() {
                    Err(VolumeContractError::InvalidMountGrantScope)
                } else {
                    Ok(Self(value))
                }
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

mount_identifier!(
    VolumeMountGrantId,
    "Stable identifier for one explicit immutable typed mount grant."
);
mount_identifier!(
    VolumeMountRevocationId,
    "Stable identifier for one permanent mount-grant revocation."
);

/// Exact immutable consumer scope; a declaration or this value alone grants nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "VolumeMountScopeWire")]
pub struct VolumeMountScope {
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    release_agent_id: ReleaseAgentId,
    slot: CapabilitySlotKey,
    volume_id: VolumeId,
    access_mode: VolumeAccessMode,
    release_contract_hash: AuthorityHash,
}

impl VolumeMountScope {
    /// Pins one exact published declaration and consumer selection.
    ///
    /// # Errors
    ///
    /// Rejects nil durable identifiers; storage must additionally prove exact
    /// declaration, binding, project, hash, and live authorization equality.
    pub fn new(
        instance_id: AgentInstanceId,
        revision_id: AgentInstanceRevisionId,
        release_agent_id: ReleaseAgentId,
        slot: CapabilitySlotKey,
        volume_id: VolumeId,
        access_mode: VolumeAccessMode,
        release_contract_hash: AuthorityHash,
    ) -> Result<Self, VolumeContractError> {
        if [
            instance_id.as_uuid(),
            revision_id.as_uuid(),
            release_agent_id.as_uuid(),
            volume_id.as_uuid(),
        ]
        .iter()
        .any(Uuid::is_nil)
        {
            return Err(VolumeContractError::InvalidMountGrantScope);
        }
        Ok(Self {
            instance_id,
            revision_id,
            release_agent_id,
            slot,
            volume_id,
            access_mode,
            release_contract_hash,
        })
    }
    /// Exact consumer instance.
    #[must_use]
    pub const fn instance_id(&self) -> AgentInstanceId {
        self.instance_id
    }
    /// Exact immutable consumer revision.
    #[must_use]
    pub const fn revision_id(&self) -> AgentInstanceRevisionId {
        self.revision_id
    }
    /// Exact exported release agent.
    #[must_use]
    pub const fn release_agent_id(&self) -> ReleaseAgentId {
        self.release_agent_id
    }
    /// Exact declared symbolic slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }
    /// Exact selected resource.
    #[must_use]
    pub const fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
    /// Exact declared mode; read-only and read-write do not substitute.
    #[must_use]
    pub const fn access_mode(&self) -> VolumeAccessMode {
        self.access_mode
    }
    /// Frozen runtime contract hash, unaffected by granting authority.
    #[must_use]
    pub const fn release_contract_hash(&self) -> AuthorityHash {
        self.release_contract_hash
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeMountScopeWire {
    instance_id: AgentInstanceId,
    revision_id: AgentInstanceRevisionId,
    release_agent_id: ReleaseAgentId,
    slot: CapabilitySlotKey,
    volume_id: VolumeId,
    access_mode: VolumeAccessMode,
    release_contract_hash: AuthorityHash,
}
impl TryFrom<VolumeMountScopeWire> for VolumeMountScope {
    type Error = VolumeContractError;
    fn try_from(value: VolumeMountScopeWire) -> Result<Self, Self::Error> {
        Self::new(
            value.instance_id,
            value.revision_id,
            value.release_agent_id,
            value.slot,
            value.volume_id,
            value.access_mode,
            value.release_contract_hash,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_and_identifier_serde_preserve_validation() {
        assert!(VolumeMountGrantId::from_uuid(Uuid::nil()).is_err());
        assert!(
            serde_json::from_str::<VolumeMountGrantId>("\"00000000-0000-0000-0000-000000000000\"")
                .is_err()
        );
        let scope = VolumeMountScope::new(
            AgentInstanceId::new(),
            AgentInstanceRevisionId::new(),
            ReleaseAgentId::new(),
            CapabilitySlotKey::parse("data").unwrap(),
            VolumeId::new(),
            VolumeAccessMode::ReadOnly,
            AuthorityHash::from_bytes([1; 32]),
        )
        .unwrap();
        let mut wire = serde_json::to_value(&scope).unwrap();
        assert_eq!(
            serde_json::from_value::<VolumeMountScope>(wire.clone()).unwrap(),
            scope
        );
        wire["revision_id"] = serde_json::json!(Uuid::nil());
        assert!(serde_json::from_value::<VolumeMountScope>(wire).is_err());
    }
}
