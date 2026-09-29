//! Pure resolution of UI gateway references to exact release-agent IDs.
//!
//! This module only binds validated source declarations to caller-supplied
//! release identities. It does not create gateway rows or infer identities.

mod model;
mod resolver;

#[cfg(test)]
mod tests;

pub use model::{
    GatewayReferenceKind, GatewayResolutionError, ReleaseAgentBinding, ResolvedApiBinding,
    ResolvedGatewayUi, ResolvedGatewayUis, ResolvedManagedService,
};
pub use resolver::resolve_gateway_uis;
