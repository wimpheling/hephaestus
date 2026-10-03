//! Checked data and fresh-identity boundary for qualified instance execution.
//!
//! Activation is one explicit qualified recipe birth transition, version 1 to 2.
//! First-profile eligibility requires a prepared whole recipe deployment and
//! original instance-creation lineage. Historical generic imports remain unsupported. Invocation requires
//! an already open gate and never activates implicitly.
//! These constructors confer no authority or physical readiness/ownership proof.

mod commands;
mod configuration;
mod ids;
mod results;

pub use commands::{ActivateInstance, InvokeInstance};
pub use configuration::InstanceExecutionConfiguration;
pub use ids::{InstanceActivationId, InstanceInvocationId};
pub use results::{InstanceActivationAdmission, InstanceInvocationAdmission};

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use thiserror::Error;

/// Closed errors for checked input and qualified execution admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum InstanceExecutionError {
    /// An identity is nil or otherwise invalid.
    #[error("instance execution identity is invalid")]
    InvalidIdentifier,
    /// The first profile requires creation version 1 and activated version 2.
    #[error("instance execution version is invalid")]
    InvalidVersion,
    /// Current user status or exact required permission is denied.
    #[error("instance execution is not authorized")]
    AuthorizationDenied,
    /// Configured runtime or positive prepared recipe birth eligibility is unsupported.
    #[error("instance execution profile is unsupported")]
    Unsupported,
    /// Referenced immutable instance/source evidence is unavailable.
    #[error("instance execution evidence is unavailable")]
    Unavailable,
    /// The gate, lifecycle, permanent closure or readiness forbids admission.
    #[error("instance is closed or unready for execution")]
    Closed,
    /// Current revision or instance version no longer matches the exact command.
    #[error("instance execution revision or version changed")]
    StaleInstance,
    /// Stable identity was reused with different immutable input or actor.
    #[error("instance execution input conflicts")]
    InputConflict,
    /// Current server configuration differs from the recorded admission.
    #[error("instance execution configuration changed")]
    ConfigurationConflict,
    /// Storage/cancellation leaves the durable outcome unknown.
    ///
    /// This cannot authorize replacement IDs or prove absence/no effects.
    #[error("instance execution outcome requires reconciliation")]
    OutcomeUncertain,
}

/// Fresh authenticated application admission, independent of runtime providers.
///
/// Implementations must validate the real active caller and current permission
/// BEFORE replay, and recheck under authoritative locks. Immutable actor/request
/// history is data; it must never be turned into an authenticated identity.
/// Server composition supplies runtime scope/config; these commands cannot.
#[async_trait]
pub trait InstanceExecutionService: Send + Sync {
    /// Activates one positively prepared recipe instance after exact readiness.
    ///
    /// Requires active caller InstanceCanManage and exact SourceCanUse, genuine
    /// original birth lineage, current revision and one-time version 1 -> 2 CAS.
    /// Gate opening, command/audits/event/outbox must commit atomically. Replay
    /// requires current active/open/version 2 state and returns the original
    /// receipt without updating or reopening a subsequently closed gate.
    ///
    /// # Errors
    /// Rejects denial, unsupported birth/config, unready/closed or stale instance,
    /// conflicting immutable input and uncertain storage outcomes.
    async fn activate_instance(
        &self,
        identity: &AuthenticatedIdentity,
        command: ActivateInstance,
    ) -> Result<InstanceActivationAdmission, InstanceExecutionError>;

    /// Admits one distinct Invocation of the already active immutable revision.
    ///
    /// Requires active caller InstanceCanExecute and exact SourceCanUse, true
    /// gate, exact revision and current configured scope/profile. It never opens
    /// the gate, uses Git, overrides parameters/policy or fabricates grants.
    /// Request/Run/command/audits/start outbox commit together before provider IO.
    /// Replay rechecks current authority/config and returns the original Run.
    /// No transaction may span a provider call. Cancellation proves no absence.
    ///
    /// # Errors
    /// Rejects denial, unsupported profile, closed/stale instance, configuration
    /// or immutable input conflict and uncertain admission outcomes.
    async fn invoke_instance(
        &self,
        identity: &AuthenticatedIdentity,
        command: InvokeInstance,
    ) -> Result<InstanceInvocationAdmission, InstanceExecutionError>;
}

#[cfg(test)]
mod tests;
