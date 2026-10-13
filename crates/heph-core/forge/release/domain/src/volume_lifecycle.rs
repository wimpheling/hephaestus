//! Instance attachment profile, independent of immutable revision completeness.

use serde::{Deserialize, Serialize};

/// Immutable attachment profile selected when an instance is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceVolumeMode {
    /// Historical scalar state-volume runtime behavior.
    Legacy,
    /// Exact named declarations, bindings and explicit mount grants.
    Named,
}
