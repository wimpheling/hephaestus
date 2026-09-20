//! Provider-neutral static distribution UI installation commands.

use release_domain::{
    ReleaseId, UiInstallationCallerKey, UiInstallationGenerationId, UiInstallationId,
    UiInstallationState, UiInstallationTarget, ui::UiKey,
};
use thiserror::Error;
use uuid::Uuid;

/// A command to install one published static UI with no API bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStaticUi {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Project or repository owner of the installation.
    pub target: UiInstallationTarget,
    /// Published release containing the UI declaration and artifacts.
    pub release_id: ReleaseId,
    /// Exact published UI declaration key.
    pub ui_key: UiKey,
}

/// Result of a committed static UI installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallStaticUiResult {
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Immutable first-generation identity.
    pub generation_id: UiInstallationGenerationId,
    /// Committed lifecycle state.
    pub state: UiInstallationState,
    /// Actor-bound occurrence/idempotency identity used by the event ledger.
    pub idempotency_id: Uuid,
}

/// Stable, transport-neutral failures for static UI installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UiInstallationError {
    /// The owner or release could not be read in the authorized transaction.
    #[error("UI installation authority is unavailable")]
    Unavailable,
    /// The actor lacks current owner or release-use authority.
    #[error("UI installation is not authorized")]
    PermissionDenied,
    /// The requested release UI is not a supported static zero-API UI.
    #[error("published UI declaration is invalid or unsupported")]
    InvalidOrUnsupported,
    /// An active installation already owns this target and key.
    #[error("an active UI installation already exists")]
    AlreadyInstalled,
    /// A caller key was reused with different canonical input.
    #[error("UI installation idempotency key conflicts with prior input")]
    IdempotencyConflict,
}
