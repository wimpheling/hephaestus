//! Provider-neutral repository workspace and agent-result contracts.

use async_trait::async_trait;
use run_domain::Run;
use runtime_types::RunId;
use std::{error::Error, fmt};
use uuid::Uuid;
use vm_trait::VmMount;

macro_rules! id_type {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a new opaque identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Constructs an identifier from its durable UUID.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the durable UUID.
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

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

id_type!(WorkspaceId, "Opaque identifier for one run workspace.");
id_type!(
    ResultId,
    "Opaque identifier for one finalized agent result."
);
id_type!(
    ArtifactId,
    "Opaque identifier for one durable result artifact."
);

/// A workspace prepared for VM attachment.
#[derive(Debug, Clone)]
pub struct PreparedWorkspace {
    /// Durable workspace identifier, absent when workspace mounting is disabled.
    pub id: Option<WorkspaceId>,
    /// Provider-neutral mounts to append to the VM specification.
    pub mounts: Vec<VmMount>,
}

impl PreparedWorkspace {
    /// Returns a disabled workspace with no guest mounts.
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            id: None,
            mounts: Vec::new(),
        }
    }
}

/// One controlled Git result published by the trusted host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedResult {
    /// Durable result identifier.
    pub id: ResultId,
    /// Fully qualified controlled result ref.
    pub result_ref: String,
    /// Host-created result commit object ID.
    pub result_commit: String,
    /// Host-created result tree object ID.
    pub result_tree: String,
}

/// Provider-neutral workspace lifecycle failure.
#[derive(Debug, thiserror::Error)]
#[error("workspace lifecycle operation failed: {0}")]
pub struct WorkspaceError(#[source] Box<dyn Error + Send + Sync>);

impl WorkspaceError {
    /// Wraps an implementation-specific failure.
    #[must_use]
    pub fn operation(error: impl Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }
}

/// Trusted host boundary for run workspace and result lifecycle operations.
#[async_trait]
pub trait RunWorkspaceManager: Send + Sync + 'static {
    /// Materializes the exact input commit and returns approved guest mounts.
    async fn prepare(&self, run: &Run) -> Result<PreparedWorkspace, WorkspaceError>;

    /// Seals, imports, and publishes one finalized result.
    async fn finalize(
        &self,
        run: &Run,
        message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError>;

    /// Removes an unfinalized active workspace without publishing a result.
    async fn abandon(&self, run_id: RunId) -> Result<(), WorkspaceError>;

    /// Reconciles incomplete seals and Git ref publications after restart.
    async fn recover(&self) -> Result<usize, WorkspaceError>;
}

/// Workspace manager used when no repository workspace is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisabledWorkspaceManager;

#[async_trait]
impl RunWorkspaceManager for DisabledWorkspaceManager {
    async fn prepare(&self, _run: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        Ok(PreparedWorkspace::disabled())
    }

    async fn finalize(
        &self,
        _run: &Run,
        _message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError> {
        Ok(None)
    }

    async fn abandon(&self, _run_id: RunId) -> Result<(), WorkspaceError> {
        Ok(())
    }

    async fn recover(&self) -> Result<usize, WorkspaceError> {
        Ok(0)
    }
}
