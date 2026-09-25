//! Controlled publication of administrator-owned OCI layouts to Zot.
//!
//! This adapter never accepts a caller-selected registry, repository, command,
//! or credential.  The durable [`PublicationIntent`] defines the sole remote
//! subject; the configured registry authority and local roots form the other
//! half of that trust boundary.

mod command;
mod config;
mod constants;
mod errors;
mod layout;
mod paths;
mod publisher;
mod read_client;
mod remote;
mod verification;

pub use command::SystemCommandRunner;
pub use config::{
    CommandOutput, CommandRunner, CommandSpec, PublicationEvidenceFiles, PublicationMaterial,
    PublisherConfiguration,
};
pub use errors::{CommandRunnerError, PublisherError, RegistryReadError};
pub use publisher::ControlledOciPublisher;
pub use read_client::{HttpRegistryReadClient, LazyHttpRegistryReadClient, RegistryReadClient};

#[cfg(test)]
mod tests;
