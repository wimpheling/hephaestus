//! Controlled query/effect ordering, not a `PostgreSQL` authorization proof.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Controlled {
    project: ProjectId,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl Engine for Controlled {
    async fn project(
        &self,
        _: &AuthenticatedIdentity,
        _: DeploymentId,
    ) -> Result<ProjectId, RecipeApplicationError> {
        Ok(self.project)
    }
    async fn resume(
        &self,
        _: &AuthenticatedIdentity,
        _: CommandIdentity,
        _: DeploymentId,
    ) -> Result<RecipeInstallation, RecipeApplicationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(recipe_application::DeploymentError::Unavailable.into())
    }
}
#[tokio::test]
async fn foreign_expected_project_denies_before_resume_effect() {
    let identity = super::super::tests::identity();
    let plan = super::super::tests::plan();
    let original =
        CommandIdentity::from_identity(&identity, recipe_application::DeploymentOperation::Install)
            .unwrap();
    let engine = Controlled {
        project: plan.project_id(),
        calls: AtomicUsize::new(0),
    };
    let foreign = ProjectId::from_uuid(uuid::Uuid::new_v4());
    assert!(matches!(
        checked(&engine, &identity, original, plan.id(), foreign).await,
        Err(RecipeApplicationError::Deployment(
            recipe_application::DeploymentError::AuthorizationDenied
        ))
    ));
    assert_eq!(engine.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        checked(&engine, &identity, original, plan.id(), plan.project_id()).await,
        Err(RecipeApplicationError::Deployment(
            recipe_application::DeploymentError::Unavailable
        ))
    ));
    assert_eq!(engine.calls.load(Ordering::SeqCst), 1);
}
