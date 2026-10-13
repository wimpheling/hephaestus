//! Checked comparison/request data; actual planning and authorization stay in the engine.
use super::AppError;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{CommandIdentity, DeploymentId, DeploymentOperation, PlanningRequest};

pub enum Request {
    Install {
        identity: AuthenticatedIdentity,
        request: Box<PlanningRequest>,
        original: CommandIdentity,
    },
    Resume {
        identity: AuthenticatedIdentity,
        original: CommandIdentity,
        deployment: DeploymentId,
        expected_project: heph_forge::ProjectId,
    },
}
impl Request {
    pub fn install(
        identity: AuthenticatedIdentity,
        request: PlanningRequest,
    ) -> Result<Self, AppError> {
        let original = CommandIdentity::from_identity(&identity, DeploymentOperation::Install)
            .map_err(crate::managed_backend::custody::failure)?;
        Ok(Self::Install {
            identity,
            request: Box::new(request),
            original,
        })
    }
    pub fn resume(
        identity: AuthenticatedIdentity,
        original: CommandIdentity,
        deployment: DeploymentId,
        expected_project: heph_forge::ProjectId,
    ) -> Result<Self, AppError> {
        if expected_project.as_uuid().is_nil() {
            return Err(crate::managed_backend::custody::failure(
                recipe_application::DeploymentError::InvalidIdentifier,
            ));
        }
        original
            .validate(&identity, DeploymentOperation::Install)
            .map_err(crate::managed_backend::custody::failure)?;
        Ok(Self::Resume {
            identity,
            original,
            deployment,
            expected_project,
        })
    }
    #[cfg(test)]
    pub const fn identity(&self) -> &AuthenticatedIdentity {
        match self {
            Self::Install { identity, .. } | Self::Resume { identity, .. } => identity,
        }
    }
    pub fn deployment(&self) -> DeploymentId {
        match self {
            Self::Install { request, .. } => request.id(),
            Self::Resume { deployment, .. } => *deployment,
        }
    }
    pub const fn original(&self) -> CommandIdentity {
        match self {
            Self::Install { original, .. } | Self::Resume { original, .. } => *original,
        }
    }
    pub fn resumes(&self, original: CommandIdentity) -> bool {
        matches!(self, Self::Resume { original: saved, .. } if *saved == original)
    }
}
