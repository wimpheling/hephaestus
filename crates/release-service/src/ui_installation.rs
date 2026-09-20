//! Provider-neutral distribution UI installation commands.

use forge_domain::OrganizationId;
use release_domain::{
    ReleaseId, UiInstallationCallerKey, UiInstallationGenerationId, UiInstallationId,
    UiInstallationState, UiInstallationTarget, ui::UiKey,
};
use thiserror::Error;
use uuid::Uuid;

/// A command to install one published UI and all of its immutable bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallUi {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Project, repository, or organization owner of the installation.
    pub target: UiInstallationTarget,
    /// Published release containing the UI declaration and artifacts.
    pub release_id: ReleaseId,
    /// Exact published UI declaration key.
    pub ui_key: UiKey,
    /// Optional tenant expectation supplied by the external RPC boundary.
    /// Static compatibility callers leave this unset.
    pub expected_organization_id: Option<OrganizationId>,
}

/// Result of a committed UI installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallUiResult {
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Immutable first-generation identity.
    pub generation_id: UiInstallationGenerationId,
    /// Committed lifecycle state.
    pub state: UiInstallationState,
    /// Actor-bound occurrence/idempotency identity used by the event ledger.
    pub idempotency_id: Uuid,
}

/// A command that activates a new immutable generation for an installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivateUiInstallation {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Optional compare-and-swap expectation for the current generation.
    pub expected_generation_id: Option<UiInstallationGenerationId>,
    /// Published release containing the replacement UI declaration and artifacts.
    pub release_id: ReleaseId,
    /// Exact published UI declaration key; must match the installation identity.
    pub ui_key: UiKey,
}

/// A command that rolls an installation back to a selected release as a new
/// immutable generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackUiInstallation {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Optional compare-and-swap expectation for the current generation.
    pub expected_generation_id: Option<UiInstallationGenerationId>,
    /// Published release to pin in the new generation.
    pub release_id: ReleaseId,
    /// Exact published UI declaration key; must match the installation identity.
    pub ui_key: UiKey,
}

/// Result of a committed activation or rollback generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiInstallationGenerationResult {
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Owner event scope needed to retrieve the committed mutation receipt.
    pub receipt_scope: UiInstallationReceiptScope,
    /// Newly created immutable generation identity.
    pub generation_id: UiInstallationGenerationId,
    /// Committed lifecycle state.
    pub state: UiInstallationState,
    /// Actor-bound occurrence/idempotency identity used by the event ledger.
    pub idempotency_id: Uuid,
}

/// Owner event scope used by the transport receipt reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiInstallationReceiptScope {
    /// Event aggregate type (`organization`, `project`, or `repository`).
    pub aggregate_type: &'static str,
    /// Event primary scope kind used by the receipt reader.
    pub primary_scope_kind: &'static str,
}

/// A compatibility command for one published static UI with no API bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStaticUi {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Project, repository, or organization owner of the installation.
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

/// A command that disables one retained UI installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableUiInstallation {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Optional compare-and-swap expectation for the current generation.
    pub expected_generation_id: Option<UiInstallationGenerationId>,
}

/// A command that terminally removes one retained UI installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveUiInstallation {
    /// Actor-bound caller idempotency key.
    pub caller_key: UiInstallationCallerKey,
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Optional compare-and-swap expectation for the current generation.
    pub expected_generation_id: Option<UiInstallationGenerationId>,
}

/// Result of a committed disable or remove command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiInstallationLifecycleResult {
    /// Stable installation identity.
    pub installation_id: UiInstallationId,
    /// Owner event scope needed to retrieve the committed mutation receipt.
    pub receipt_scope: UiInstallationReceiptScope,
    /// The retained current generation; lifecycle changes do not create one.
    pub generation_id: UiInstallationGenerationId,
    /// Committed lifecycle state.
    pub state: UiInstallationState,
    /// Actor-bound occurrence/idempotency identity used by the event ledger.
    pub idempotency_id: Uuid,
}

/// Stable, transport-neutral failures for UI installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UiInstallationError {
    /// The owner or release could not be read in the authorized transaction.
    #[error("UI installation authority is unavailable")]
    Unavailable,
    /// The actor lacks current owner or release-use authority.
    #[error("UI installation is not authorized")]
    PermissionDenied,
    /// The caller's expected organization does not match the locked owner.
    #[error("UI installation organization does not match the target owner")]
    OrganizationMismatch,
    /// The requested release UI is not a supported static zero-API UI.
    #[error("published UI declaration is invalid or unsupported")]
    InvalidOrUnsupported,
    /// An active installation already owns this target and key.
    #[error("an active UI installation already exists")]
    AlreadyInstalled,
    /// A caller key was reused with different canonical input.
    #[error("UI installation idempotency key conflicts with prior input")]
    IdempotencyConflict,
    /// The caller supplied a stale expected generation.
    #[error("UI installation generation no longer matches the expected generation")]
    GenerationConflict,
    /// The requested lifecycle transition is not valid for the installation.
    #[error("UI installation lifecycle transition is invalid")]
    InvalidTransition,
}
