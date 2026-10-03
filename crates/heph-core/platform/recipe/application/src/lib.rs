//! Pure deployment intent planning and durable repository contracts.
//!
//! These values convey no authorization. Implementations must load authoritative
//! release/resource catalogs and recheck live permissions before replay or effects.
//!
//! Validated intent cannot be hydrated through unchecked deserialization:
//!
//! ```compile_fail
//! let _: recipe_application::DeploymentIntent = serde_json::from_str("{}").unwrap();
//! ```

mod active_effect;
mod commands;
mod errors;
mod execution_preparation;
mod execution_preparation_repository;
mod existing_instance_closure;
mod ids;
mod instance_closure;
mod instance_safety;
mod intent;
mod owned_partial_retention;
mod owned_ready_retention;
mod planning;
mod planning_observations;
mod planning_request;
mod remove_resume;
mod repository;
mod states;

pub use active_effect::ActiveDeploymentEffectRepository;
pub use commands::*;
pub use errors::DeploymentError;
pub use execution_preparation::{
    DeploymentExecutionPreparation, DeploymentExecutionProfile, MAX_EXECUTION_PREPARATION_BYTES,
    PreparedInstanceImport, PreparedVolumeSelection,
};
pub use execution_preparation_repository::{
    DeploymentExecutionPreparationRepository, ExecutionPreparationAdmission,
    ExecutionPreparationReceipt,
};
pub use existing_instance_closure::{
    ExistingInstanceClosureRepository, ExistingInstanceClosureRequest,
};
pub use ids::{
    DeploymentAttemptId, DeploymentCommandId, DeploymentId, DeploymentKey, MAX_DEPLOYMENT_KEY_BYTES,
};
pub use instance_closure::{
    PreparedInstanceClosureAdmission, PreparedInstanceClosureRepository,
    PreparedInstanceClosureRequest,
};
pub use instance_safety::{
    InstanceSafetyInventory, InstanceSafetyWork, InstanceSafetyWorkKind,
    PreparedInstanceSafetyObservation, PreparedInstanceSafetyRepository,
    PreparedInstanceSafetyState, PreparedInstanceSafetyTarget,
};
pub use intent::{DeploymentIntent, PlannedResource, PlannedResourceIdentity, ResourceOwnership};
pub use owned_partial_retention::{
    OwnedPartialRetentionRepository, PartialCreationRetentionPlan, PreparedOwnedVolumeCreation,
    RetainPartialCreation,
};
pub use owned_ready_retention::{
    OwnedReadyRemoveResume, OwnedReadyRetentionPlan, OwnedReadyRetentionRepository,
    ReopenRetainedOwnedVolume, RetainOwnedReady,
};
pub use planning::{PlanningCatalog, PlanningCatalogSnapshot, RecipePlan, RecipePlanner};
pub use planning_observations::{PlatformPolicyObservation, SourceObservation};
pub use planning_request::PlanningRequest;
pub use repository::*;
pub use states::*;

#[cfg(test)]
mod tests;

pub use remove_resume::{
    OriginalInstanceClosure, RemoveContinuation, RemoveResumeAdmission, RemoveResumeConfiguration,
};
