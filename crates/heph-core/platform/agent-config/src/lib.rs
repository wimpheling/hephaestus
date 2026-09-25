//! Versioned `agent.toml` parsing, validation, and trigger matching.

mod model_build;
mod model_capabilities;
mod model_core;
mod model_gateway;
mod model_parameters;
mod model_runtime;
mod parser;
mod validation_capabilities;
mod validation_core;
mod validation_images;
mod validation_parameters;
mod validation_shared;
mod validation_v2;

pub mod build_identity;
pub mod ui;
pub use ui::{
    ParsedRepositoryUis, parse_repository_uis, validate_repository_uis_against_gateways,
    validate_ui_route_collisions,
};

pub use model_build::{
    BuildArtifact, BuildArtifactKind, NetworkConfig, NetworkProfile, PublicationConfig,
    PublicationMode, ResourceLimits, ResultConfig, StateVolume, WorkspaceMount,
};
pub use model_capabilities::{
    CapabilitySlotDeclaration, GitBranchUpdateDeclaration, GitCapabilityDeclaration,
    GitTransferLimitDeclaration,
};
pub use model_core::{
    AgentConfig, AgentIdentity, BuildConfig, GuestConfig, ImageSelection,
    RepositoryOciImageBuildConfig, RepositoryOciImageConfig, RepositoryOciImagesConfig,
};
pub use model_gateway::{
    RepositoryGatewayConfig, RepositoryGatewayMailboxPublicationSlot, RepositoryGatewayRouteConfig,
    RepositoryGatewayServiceConfig, RepositoryGatewaysConfig,
};
pub use model_parameters::{
    ParameterDeclaration, ParameterDefault, ParameterType, SecretSlotDeclaration,
};
pub use model_runtime::{
    ConfigHash, Diagnostic, ParsedConfig, ParsedRepositoryGateways, ParsedRepositoryOciImages,
    TriggerConfig, UpdateHookConfig,
};
pub use model_runtime::{SecretDeliveryMode, SecretPhase};
pub(crate) use parser::hash;
pub use parser::{
    canonical_repository_gateways, parse, parse_repository_gateways, parse_repository_oci_images,
};
pub(crate) use validation_images::validate_repository_gateways;
/// Reusable release and typed-instance configuration version.
pub const REUSABLE_RELEASE_VERSION: u32 = 2;
/// Schema version for a repository's OCI image manifest.
pub const REPOSITORY_OCI_IMAGES_VERSION: u32 = 1;
/// Schema version for a repository's gateway manifest.
pub const REPOSITORY_GATEWAYS_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    mod basic;
    mod capabilities;
    mod images;
    mod release;
    mod repositories;
    mod support;
}
