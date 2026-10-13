//! Project comparison runs inside the already owning FIFO engine job.
use crate::recipe_engine::{PreparedRecipeApplication, RecipeApplicationError, RecipeInstallation};
use heph_forge::ProjectId;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandIdentity, DeploymentId};

#[async_trait::async_trait]
trait Engine: Sync {
    async fn project(
        &self,
        identity: &AuthenticatedIdentity,
        deployment: DeploymentId,
    ) -> Result<ProjectId, RecipeApplicationError>;
    async fn resume(
        &self,
        identity: &AuthenticatedIdentity,
        original: CommandIdentity,
        deployment: DeploymentId,
    ) -> Result<RecipeInstallation, RecipeApplicationError>;
}
#[async_trait::async_trait]
impl Engine for PreparedRecipeApplication {
    async fn project(
        &self,
        identity: &AuthenticatedIdentity,
        deployment: DeploymentId,
    ) -> Result<ProjectId, RecipeApplicationError> {
        Ok(self
            .inspect(identity, deployment)
            .await?
            .ledger
            .intent
            .project_id())
    }
    async fn resume(
        &self,
        identity: &AuthenticatedIdentity,
        original: CommandIdentity,
        deployment: DeploymentId,
    ) -> Result<RecipeInstallation, RecipeApplicationError> {
        Self::resume(self, identity, original, deployment).await
    }
}
pub(super) async fn run(
    engine: &PreparedRecipeApplication,
    identity: &AuthenticatedIdentity,
    original: CommandIdentity,
    deployment: DeploymentId,
    expected: ProjectId,
) -> Result<RecipeInstallation, RecipeApplicationError> {
    checked(engine, identity, original, deployment, expected).await
}
async fn checked(
    engine: &impl Engine,
    identity: &AuthenticatedIdentity,
    original: CommandIdentity,
    deployment: DeploymentId,
    expected: ProjectId,
) -> Result<RecipeInstallation, RecipeApplicationError> {
    if engine.project(identity, deployment).await? != expected {
        return Err(recipe_application::DeploymentError::AuthorizationDenied.into());
    }
    engine.resume(identity, original, deployment).await
}
#[cfg(test)]
#[path = "resume/tests.rs"]
mod tests;
