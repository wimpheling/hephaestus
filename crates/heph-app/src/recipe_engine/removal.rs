//! Owning installed managed recipe removal, with exact durable terminal replay.
mod admission;
mod completion;
mod context;
mod custody;
mod instances;
mod retained;
#[cfg(test)]
mod tests;
mod validation;
mod volumes;

use crate::recipe::ready_retention::ReadyRetentionError;
use crate::{
    AppError, ManagedInvocationController, PreparedInstanceDrainError,
    PreparedInstanceTerminalError,
};
use context::Context;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandReceipt, DeploymentError, DeploymentId, RemoveDeployment};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Each original error is retained; an error never declares absence or rollback.
#[derive(Debug, thiserror::Error)]
pub enum RecipeRemovalError {
    /// Current authorization or durable original/history qualification failed.
    #[error(transparent)]
    Deployment(#[from] DeploymentError),
    /// The SAME native owners/configuration changed.
    #[error(transparent)]
    Custody(#[from] AppError),
    /// Actual worker/physical volume evidence failed.
    #[error(transparent)]
    Volume(#[from] volume_trait::VolumeError),
    /// Actual authorized instance status/CAS query failed.
    #[error(transparent)]
    Release(#[from] release_postgres::ReleaseServiceError),
    /// The real owning Drain has not completed.
    #[error(transparent)]
    Drain(Arc<PreparedInstanceDrainError>),
    /// The real owning Detach/Delete has not completed.
    #[error(transparent)]
    Terminal(Arc<PreparedInstanceTerminalError>),
    /// Actual Ready retention remains owned and held.
    #[error(transparent)]
    Retention(#[from] ReadyRetentionError),
    /// Unsupported partial/foreign/policy histories cannot become completion.
    #[error("recipe removal original or policy remains unsupported/unproven")]
    Unsupported,
    /// Reply loss is uncertainty, never a fabricated terminal result.
    #[error("recipe removal outcome unknown; original owner retained")]
    OutcomeUnknown,
}

/// Safe registration status; this conveys no provider/physical/dispatch authority.
#[derive(Debug, Clone)]
pub struct RecipeRemovalPending {
    /// Exact deployment whose intake is serialized.
    pub deployment_id: DeploymentId,
    /// Last submitted actual current manager command; saved actors are only data.
    pub current_command: recipe_application::CommandIdentity,
    /// An expected error awaits explicit current-auth retry.
    pub held: bool,
}

/// Owns all real instance/volume/final services from ONE original controller.
///
/// Callers receive no provider/root/guard/pool or task abort handle. Explicit
/// service retention is mandatory while held; last-owner/runtime loss is not a
/// cleanup/absence guarantee. No readiness, RPC, root initialization or VM launch
/// is activated by this opt-in coordinator.
pub struct PreparedRecipeRemovalCoordinator {
    context: Arc<context::Context>,
    pending: Arc<Mutex<BTreeMap<DeploymentId, custody::Control>>>,
}
#[derive(Clone)]
struct Request {
    identity: AuthenticatedIdentity,
    remove: RemoveDeployment,
    mode: Mode,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Original,
    Resume,
}
impl PreparedRecipeRemovalCoordinator {
    /// Derives every concrete port and owning subservice from the SAME controller.
    ///
    /// # Errors
    /// Rejects changed owners or invalid trusted composition before any IO.
    pub fn new(controller: Arc<ManagedInvocationController>) -> Result<Self, RecipeRemovalError> {
        Ok(Self {
            context: Arc::new(context::Context::new(controller)?),
            pending: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
    /// Drives an original real Remove in reverse dependency order.
    ///
    /// Registration precedes the first await; aborting the waiter does not abort
    /// the physical work. Exact original input/operations/CAS survive held retries.
    ///
    /// # Errors
    /// Current rights loss, unsupported originals or incomplete physical/durable
    /// evidence remains held without replacing its identities or guessing absence.
    pub async fn remove(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
    ) -> Result<CommandReceipt, Arc<RecipeRemovalError>> {
        custody::submit(
            self,
            Request {
                identity: identity.clone(),
                remove,
                mode: Mode::Original,
            },
        )
        .await
    }
    /// Uses a genuinely new configured current manager, never historical credentials.
    ///
    /// Cold Removed replay uses the exact saved receipt/before-CAS reader without
    /// creating a Remove or borrowing the recorded terminal publisher.
    ///
    /// # Errors
    /// Denies changed input/current authority, incomplete originals or uncertain IO.
    pub async fn resume(
        &self,
        identity: &AuthenticatedIdentity,
        remove: RemoveDeployment,
    ) -> Result<CommandReceipt, Arc<RecipeRemovalError>> {
        custody::submit(
            self,
            Request {
                identity: identity.clone(),
                remove,
                mode: Mode::Resume,
            },
        )
        .await
    }
    /// Inspects retained registration only; never returns authority or physical proof.
    #[must_use]
    pub fn pending_removals(&self) -> Vec<RecipeRemovalPending> {
        custody::inspect(&self.pending)
    }
}
