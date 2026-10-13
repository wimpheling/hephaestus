//! Closed owned-backing states and physical provider-root identity.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::VolumeCreationIdentityError;

/// Persistent physical volume-provider root identity, independent of owner scope.
///
/// A checked UUID is only a label. Provider composition must bind its durable
/// owner marker to the verified root device/inode; paths or hashes grant nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Uuid", into = "Uuid")]
pub struct VolumeRootNamespaceId(Uuid);
impl VolumeRootNamespaceId {
    /// Validates a nonnil physical provider namespace label.
    ///
    /// # Errors
    /// Rejects nil identifiers.
    pub const fn from_uuid(id: Uuid) -> Result<Self, VolumeCreationIdentityError> {
        if id.is_nil() {
            Err(VolumeCreationIdentityError)
        } else {
            Ok(Self(id))
        }
    }
    /// Returns the exact persisted provider namespace UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}
impl TryFrom<Uuid> for VolumeRootNamespaceId {
    type Error = VolumeCreationIdentityError;
    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        Self::from_uuid(value)
    }
}
impl From<VolumeRootNamespaceId> for Uuid {
    fn from(value: VolumeRootNamespaceId) -> Self {
        value.0
    }
}

/// Worker observations about the original backing birth, never runtime reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedBackingPhase {
    /// Positive complete journal proof: exact recorded inode, format never began.
    Recorded,
    /// Durable first-format intent exists; absence or expiry cannot permit format.
    FormatIntent,
    /// Authoritatively failed before format; another fresh actor operation may retry.
    FailedBeforeFormat,
    /// Exact original clean filesystem birth was observed and committed.
    Ready,
    /// Incomplete evidence requires readonly reconciliation of the same operation.
    Uncertain,
}
impl OwnedBackingPhase {
    /// Returns the closed SQL vocabulary.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::FormatIntent => "format_intent",
            Self::FailedBeforeFormat => "failed_before_format",
            Self::Ready => "ready",
            Self::Uncertain => "uncertain",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_root_namespace_cannot_be_nil() {
        assert!(VolumeRootNamespaceId::from_uuid(Uuid::nil()).is_err());
        let original = Uuid::new_v4();
        assert_eq!(
            VolumeRootNamespaceId::from_uuid(original)
                .unwrap()
                .as_uuid(),
            original
        );
    }
}
