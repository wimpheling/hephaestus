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

mod commands;
mod errors;
mod ids;
mod intent;
mod planning;
mod planning_observations;
mod planning_request;
mod repository;
mod states;

pub use commands::*;
pub use errors::DeploymentError;
pub use ids::{
    DeploymentAttemptId, DeploymentCommandId, DeploymentId, DeploymentKey, MAX_DEPLOYMENT_KEY_BYTES,
};
pub use intent::{DeploymentIntent, PlannedResource, PlannedResourceIdentity, ResourceOwnership};
pub use planning::{PlanningCatalog, PlanningCatalogSnapshot, RecipePlan, RecipePlanner};
pub use planning_observations::{PlatformPolicyObservation, SourceObservation};
pub use planning_request::PlanningRequest;
pub use repository::*;
pub use states::*;

#[cfg(test)]
mod tests;
