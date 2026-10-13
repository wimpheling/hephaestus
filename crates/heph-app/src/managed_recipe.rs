//! Same-node recipe factory and request custody; no transport or shutdown receipt.
#[path = "managed_recipe/factory.rs"]
mod factory;
#[path = "managed_recipe/jobs.rs"]
mod jobs;
#[path = "managed_recipe/ownership.rs"]
mod ownership;
#[path = "managed_recipe/request.rs"]
mod request;
#[path = "managed_recipe/resume.rs"]
mod resume;
#[cfg(test)]
#[path = "managed_recipe/tests.rs"]
mod tests;

use crate::{AppError, ManagedInvocationController, recipe_engine::RecipeInstallation};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandIdentity, DeploymentId, PlanningRequest};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::oneshot;

use super::supervisor::registry::Registry;
use jobs::State;
pub(super) use request::Request;

/// Actual engine result, shared with the retained job and its receiver.
pub type Outcome = Result<Arc<RecipeInstallation>, Arc<Failure>>;

/// Retained original failure; no variant proves resource absence or cleanup.
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error(transparent)]
    Owner(#[from] AppError),
    #[error(transparent)]
    Execution(#[from] crate::recipe_engine::RecipeApplicationError),
    #[error("managed recipe original task failed: {0}")]
    Join(#[source] tokio::task::JoinError),
    #[error("managed recipe owning task has no confirmed terminal result")]
    Unsettled,
}

/// Only the registered node can construct/admit this original backend.
pub struct ManagedRecipe {
    registry: Weak<Registry>,
    actions: Arc<dyn Actions>,
    jobs: Mutex<State>,
}

#[async_trait]
trait Actions: Send + Sync {
    async fn run(&self, request: &Request) -> Outcome;
}

/// A request budget owns only this result receiver, never the actual operation.
pub struct Ticket(oneshot::Receiver<Outcome>);
impl Ticket {
    /// Waits for the owning job; dropping this receiver leaves the job intact.
    /// # Errors
    /// Returns its actual held error or a lost result channel, never inferred success.
    pub async fn wait(self) -> Outcome {
        self.0
            .await
            .unwrap_or_else(|_| Err(Arc::new(Failure::Unsettled)))
    }
}
impl ManagedRecipe {
    pub(super) fn new(
        controller: Arc<ManagedInvocationController>,
        registry: Weak<Registry>,
    ) -> Result<Arc<Self>, AppError> {
        Ok(Arc::new(Self {
            registry,
            actions: factory::assemble(controller)?,
            jobs: Mutex::new(State::default()),
        }))
    }

    /// Admits the actual checked Install request only while this original node is Ready.
    /// # Errors
    /// Rejects lost/closed/held service custody or an invalid logical command.
    pub fn install(
        self: &Arc<Self>,
        identity: AuthenticatedIdentity,
        request: PlanningRequest,
    ) -> Result<Ticket, AppError> {
        self.admit(Request::install(identity, request)?)
    }

    /// Serializes a fresh request under the SAME original actor/Install logical identity.
    /// # Errors
    /// Denies a different manager/key or late mutation; the engine rechecks live authority.
    pub fn resume(
        self: &Arc<Self>,
        identity: AuthenticatedIdentity,
        original: CommandIdentity,
        deployment: DeploymentId,
        expected_project: heph_forge::ProjectId,
    ) -> Result<Ticket, AppError> {
        self.admit(Request::resume(
            identity,
            original,
            deployment,
            expected_project,
        )?)
    }

    fn admit(self: &Arc<Self>, request: Request) -> Result<Ticket, AppError> {
        self.registry
            .upgrade()
            .ok_or_else(|| AppError::Readiness("managed recipe original service lost".into()))?
            .admit_recipe(self, request)
    }
}
