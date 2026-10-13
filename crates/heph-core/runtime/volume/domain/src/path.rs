use serde::{Deserialize, Serialize};

use crate::{MAX_GUEST_MOUNT_PATH_BYTES, VolumeContractError};

/// A bounded absolute guest mount path with no normalization ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GuestMountPath(String);

impl GuestMountPath {
    /// Validates a canonical path outside protected guest filesystem trees.
    ///
    /// # Errors
    ///
    /// Rejects root, relative paths, empty/dot components, control characters,
    /// backslashes, oversized paths, and protected `/proc`, `/sys`, `/dev`,
    /// `/run`, `/release`, `/workspace` trees.
    pub fn parse(value: impl Into<String>) -> Result<Self, VolumeContractError> {
        let value = value.into();
        let invalid = !value.starts_with('/')
            || value.len() > MAX_GUEST_MOUNT_PATH_BYTES
            || value
                .chars()
                .any(|character| character.is_control() || character == '\\')
            || value[1..]
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
            || ["/proc", "/sys", "/dev", "/run", "/release", "/workspace"]
                .iter()
                .any(|protected| paths_overlap(protected, &value));
        if invalid {
            return Err(VolumeContractError::InvalidGuestMountPath);
        }
        Ok(Self(value))
    }

    /// Returns the validated absolute guest path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for GuestMountPath {
    type Error = VolumeContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<GuestMountPath> for String {
    fn from(value: GuestMountPath) -> Self {
        value.0
    }
}

pub fn paths_overlap(left: &str, right: &str) -> bool {
    left == right || is_descendant(left, right) || is_descendant(right, left)
}

fn is_descendant(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('/'))
}
