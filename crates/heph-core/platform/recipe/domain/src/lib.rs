//! Pure version-one resource recipe parsing, validation, and resolution.
//!
//! Validated graphs describe intent and authority ceilings; they grant no
//! permissions and perform no provider, database, or installation effects.

mod bindings;
mod catalog;
mod errors;
mod graph;
mod inputs;
mod model;
mod parser;
mod resolve;
mod resolved;

pub use catalog::{ReleaseCatalogEntry, ReleasePin};
pub use errors::{RecipeDiagnostic, RecipeError};
pub use inputs::{InputDeclaration, InputType};
pub use model::{
    InstanceResource, RecipeManifest, RecipeOutput, RecipeVolumeBinding, RemovalPolicy,
    ResourceDeclaration, ValueReference, VolumeResource, VolumeSource,
};
pub use parser::{ValidatedRecipe, parse_recipe};
pub use resolve::ExternalVolume;
pub use resolved::{ResolvedInstance, ResolvedRecipe, ResolvedResource, ResolvedVolume};

/// Supported recipe contract version.
pub const RECIPE_CONTRACT_VERSION: u32 = 1;
/// Maximum source document size accepted by the parser.
pub const MAX_RECIPE_BYTES: usize = 64 * 1024;
/// Maximum resources, inputs, or outputs in a recipe.
pub const MAX_RECIPE_ITEMS: usize = 64;
/// Maximum UTF-8 byte length of ordinary scalar string values.
pub const MAX_VALUE_BYTES: usize = 4096;

#[cfg(test)]
mod tests;
