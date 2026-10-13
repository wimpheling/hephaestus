//! Owning post-Applied-Drain instance Detach→Delete, without volume/final removal.
mod admission;
mod claims;
mod completion;
mod context;
mod custody;
mod execution;
mod retained;
#[cfg(test)]
mod tests;

use crate::{AppError, ManagedInvocationController};
use capability_domain::CapabilitySlotKey;
use context::Context;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentError, DeploymentId, RemoveDeployment, ResourceProgress};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Errors retain their original source; none establishes an effect outcome.
#[derive(Debug, thiserror::Error)]
pub enum PreparedInstanceTerminalError {
    /// Current admission, immutable comparison or protected proof failed.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// The original provider/root/configuration custody changed.
    #[error(transparent)]
    Custody(#[from] AppError),
    /// Actual Drain has not reached a positively completed predecessor.
    #[error("instance terminal work requires completed Drain; original owner retained")]
    DrainPending,
    /// Lost channel is uncertainty, not a physical or committed result.
    #[error("instance terminal outcome unknown; original owner retained")]
    OutcomeUnknown,
}

/// Safe registered ownership visibility, without provider or root handles.
#[derive(Debug, Clone)]
pub struct PreparedInstanceTerminalPending {
    /// Exact deployment registered before IO.
    pub deployment_id: DeploymentId,
    /// Validated instance resource name.
    pub resource: CapabilitySlotKey,
    /// First actual logical Remove, never reconstructed credentials.
    pub original_command: recipe_application::CommandIdentity,
    /// Positively returned active attempt, if available.
    pub attempt: Option<recipe_application::DeploymentAttemptId>,
    /// Failed work remains available for a fresh current retry.
    pub held: bool,
}

/// Concrete terminal service retaining the SAME original Invocation controller.
///
/// Registered tasks own proposals, claims, original CAS and errors independently
/// of the caller waiter. The explicit service owner keeps held retry intake alive;
/// last-owner/runtime destruction does not promise cleanup or physical absence.
/// This ends at Applied instance Delete, not volume Retain or `DeploymentRemoved`.
pub struct PreparedInstanceTerminalCoordinator {
    context: Arc<Context>,
    pending: Arc<Mutex<BTreeMap<custody::Key, custody::Control>>>,
}
#[derive(Clone)]
struct Request {
    identity: AuthenticatedIdentity,
    remove: RemoveDeployment,
    resource: CapabilitySlotKey,
    mode: Mode,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Original,
    Resume,
}

impl PreparedInstanceTerminalCoordinator {
    /// Derives all ports from the original retained controller tuple, without IO.
    ///
    /// # Errors
    /// Rejects changed owner/configuration or invalid trusted composition pins.
    pub fn new(
        controller: Arc<ManagedInvocationController>,
    ) -> Result<Self, PreparedInstanceTerminalError> {
        Ok(Self {
            context: Arc::new(Context::new(controller)?),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
    /// Advances an actual original Remove after genuine Applied Drain.
    ///
    /// Dropping this waiter does not abort admitted work. No caller CAS, pool,
    /// configuration, historical actor identity or physical receipt is accepted.
    ///
    /// # Errors
    /// Denies missing predecessor, current rights loss or unknown original outcomes.
    pub async fn finish(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
        resource: &CapabilitySlotKey,
    ) -> Result<ResourceProgress, Arc<PreparedInstanceTerminalError>> {
        custody::submit(
            self,
            Request {
                identity: identity.clone(),
                remove,
                resource: resource.clone(),
                mode: Mode::Original,
            },
        )
        .await
    }
    /// Uses a genuine current-manager admission and full143 historical projection.
    ///
    /// Saved publisher data is independent of the current authenticated principal.
    /// Inactive Applied replay observes full worker history without new claims/receipts.
    ///
    /// # Errors
    /// Denies unsupported, incomplete or foreign history and current authority loss.
    pub async fn resume(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
        resource: &CapabilitySlotKey,
    ) -> Result<ResourceProgress, Arc<PreparedInstanceTerminalError>> {
        custody::submit(
            self,
            Request {
                identity: identity.clone(),
                remove,
                resource: resource.clone(),
                mode: Mode::Resume,
            },
        )
        .await
    }
    /// Observes registered ownership only, never authorizes cleanup or dispatch.
    #[must_use]
    pub fn pending_terminal(&self) -> Vec<PreparedInstanceTerminalPending> {
        custody::inspect(&self.pending)
    }
}
