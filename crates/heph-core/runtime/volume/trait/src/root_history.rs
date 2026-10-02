//! Worker discovery of immutable physical-root comparison history.

use std::path::Path;

use async_trait::async_trait;

use crate::{VolumeError, VolumeRootNamespaceId};

/// Closed conflicts in the single-root-per-configured-host profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedVolumeRootHistoryConflict {
    /// This host's one recorded root differs from the configured root.
    RootRedirect,
    /// This host has more than one distinct root/namespace pair.
    MultipleOwnerIdentities,
}

/// Immutable history for a configured host, including permanently retired births.
///
/// These values convey no permission to create a root, adopt a file or provision
/// a resource. Absence is only the query's observation: normal startup must fail
/// held without an explicitly authorized prospective bootstrap decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnedVolumeRootHistory {
    /// No immutable owned backing purpose exists for this configured host.
    NoOwnedHistory,
    /// All recorded purposes agree with the configured root and this namespace.
    Existing {
        /// Authoritative expected label, still requiring physical marker validation.
        namespace: VolumeRootNamespaceId,
    },
    /// History prevents automatic selection of an expected physical owner.
    Conflict {
        /// Bounded conflict reason without another host's paths or resource handles.
        code: OwnedVolumeRootHistoryConflict,
    },
}

/// Worker-only readonly extension on the canonical volume metadata repository.
///
/// Implementations verify the worker role before querying all immutable purpose
/// rows for the exact configured host across projects. Repeated identical
/// root/namespace pairs coalesce. Other hosts may share the same path string and
/// are independent. Active state, retirement, timestamps and project authority
/// must not hide history. The lookup never mints or repairs an owner namespace.
#[async_trait]
pub trait VolumeRootHistoryRepository: Send + Sync + 'static {
    /// Loads this host's single-root comparison history without filesystem IO.
    ///
    /// # Errors
    /// Rejects nonworker pools, invalid bounded host/canonical UTF-8 root text,
    /// malformed persisted namespace identity or database failure.
    async fn owned_root_history(
        &self,
        configured_host: &str,
        configured_root: &Path,
    ) -> Result<OwnedVolumeRootHistory, VolumeError>;
}
