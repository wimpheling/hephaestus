use git_capability_domain::{PathChange, RefTransition};

use super::ResolvedRuntimeReceiveContext;

/// One owned changed-path fact derived from quarantined objects.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrustedPathChange {
    /// A new path.
    Addition(String),
    /// An existing path with changed content or mode.
    Modification(String),
    /// A removed path.
    Deletion(String),
    /// A rename or copy, including both authority-relevant paths.
    Rename {
        /// Original repository-relative path.
        from: String,
        /// New repository-relative path.
        to: String,
    },
}

impl TrustedPathChange {
    pub(super) fn as_borrowed(&self) -> PathChange<'_> {
        match self {
            Self::Addition(path) => PathChange::Addition(path),
            Self::Modification(path) => PathChange::Modification(path),
            Self::Deletion(path) => PathChange::Deletion(path),
            Self::Rename { from, to } => PathChange::Rename { from, to },
        }
    }
}

/// One ref command whose transition and complete path delta were inspected
/// against trusted repository state.
#[derive(Debug, Clone)]
pub struct TrustedReceiveUpdate {
    pub(super) reference: String,
    pub(super) transition: RefTransition,
    pub(super) changed_paths: Vec<TrustedPathChange>,
    pub(super) old_object: Option<String>,
}

impl TrustedReceiveUpdate {
    /// Creates one owned, trusted receive update.
    #[must_use]
    pub fn new(
        reference: impl Into<String>,
        transition: RefTransition,
        changed_paths: Vec<TrustedPathChange>,
    ) -> Self {
        Self {
            reference: reference.into(),
            transition,
            changed_paths,
            old_object: None,
        }
    }

    /// Creates a trusted update including the canonical old object observed by
    /// the host-side quarantine inspector.
    #[must_use]
    pub fn new_with_old_object(
        reference: impl Into<String>,
        transition: RefTransition,
        changed_paths: Vec<TrustedPathChange>,
        old_object: Option<String>,
    ) -> Self {
        Self {
            reference: reference.into(),
            transition,
            changed_paths,
            old_object,
        }
    }
}

/// Complete trusted facts for one atomic receive transaction.
#[derive(Debug, Clone)]
pub struct TrustedReceiveProposal {
    pub(super) context: ResolvedRuntimeReceiveContext,
    pub(super) updates: Vec<TrustedReceiveUpdate>,
    pub(super) request_bytes: u64,
    pub(super) pack_bytes: u64,
    pub(super) object_count: u32,
}

impl TrustedReceiveProposal {
    /// Creates a proposal from facts produced by trusted quarantine
    /// inspection.
    #[must_use]
    pub const fn new(
        context: ResolvedRuntimeReceiveContext,
        updates: Vec<TrustedReceiveUpdate>,
        request_bytes: u64,
        pack_bytes: u64,
        object_count: u32,
    ) -> Self {
        Self {
            context,
            updates,
            request_bytes,
            pack_bytes,
            object_count,
        }
    }

    /// Returns the immutable resolved runtime context.
    #[must_use]
    pub const fn context(&self) -> &ResolvedRuntimeReceiveContext {
        &self.context
    }

    /// Returns the complete proposed ref-command batch.
    #[must_use]
    pub fn updates(&self) -> &[TrustedReceiveUpdate] {
        &self.updates
    }
}
