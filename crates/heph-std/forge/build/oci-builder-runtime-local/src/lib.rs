//! Administrator-owned local runtime adapters for repository OCI images.
//!
//! The adapters deliberately accept only opaque durable job identities and
//! paths derived from configured roots. Repository text never becomes a host
//! path, command executable, registry credential, or scanner argument.

mod checkout;
mod config;
mod constants;
mod filesystem;
mod guest;
mod operation;
mod output;
mod publication;
mod runtime;
mod specs;

#[cfg(test)]
mod tests;

pub use config::{ForgeZotPublicationConfig, LocalOciRuntime, LocalOciRuntimeConfig};
pub use constants::{VerifiedVmOciOutput, VmOciOperationConfig};
pub use operation::VmOciOperation;
pub use output::verified_vm_publication_material;
pub use publication::{ForgeZotOciPublisher, PreparedPublicationMaterial, VmPublishedOciEngine};
