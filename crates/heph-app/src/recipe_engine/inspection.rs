use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandReceipt, DeploymentAdmission, DeploymentExecutionPreparation,
    DeploymentExecutionPreparationRepository, DeploymentId, DeploymentSnapshot,
};
use release_service::InstanceActivationAdmission;

use super::{PreparedRecipeApplication, RecipeApplicationError, validation};

/// Read projection of runtime activation, independent of installation readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationInspection {
    /// No dedicated authoritative activation reader is injected in this slice.
    ///
    /// Neither absence of a Run nor a Ready ledger can prove inactive/active state.
    Unsupported,
}

/// Authorized historical state without provider handles or reusable grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRecipeInspection {
    /// Current install/removal ledger, including held original attempts.
    pub ledger: DeploymentSnapshot,
    /// Original complete inputs, if already prepared; conveys no live source rights.
    pub preparation: Option<DeploymentExecutionPreparation>,
    /// Explicitly unknown activation projection.
    pub activation: ActivationInspection,
}

/// Separate result of one freshly authorized activation attempt.
#[derive(Debug)]
pub struct RecipeActivationAttempt {
    /// Exact saved instance resource.
    pub resource: CapabilitySlotKey,
    /// A real admission, or a held result; never inferred from installation Ready.
    pub result: Result<InstanceActivationAdmission, RecipeApplicationError>,
}

/// Completed installation ledger and separate activation attempts.
#[derive(Debug)]
pub struct RecipeInstallation {
    /// Original freshly authorized admission, including authoritative replay disposition.
    pub admission: DeploymentAdmission,
    /// Terminal receipt returned by this call; None on previously completed replay.
    ///
    /// A replay retains the original admission receipt rather than inventing the
    /// unavailable historical pre-finish CAS fingerprint.
    pub completion: Option<CommandReceipt>,
    /// Reloaded current installation progress.
    pub ledger: DeploymentSnapshot,
    /// Actual admissions returned in this invocation, not a future live-state claim.
    pub activations: Vec<RecipeActivationAttempt>,
}

pub async fn read(
    application: &PreparedRecipeApplication,
    identity: &AuthenticatedIdentity,
    deployment: DeploymentId,
) -> Result<PreparedRecipeInspection, RecipeApplicationError> {
    let ledger = application.repository.inspect(identity, deployment).await?;
    let preparation = application
        .preparation
        .inspect_execution_preparation(identity, deployment)
        .await?;
    if let Some(prepared) = &preparation {
        validation::preparation(&ledger.intent, prepared.command(), prepared)?;
    }
    Ok(PreparedRecipeInspection {
        ledger,
        preparation,
        activation: ActivationInspection::Unsupported,
    })
}
