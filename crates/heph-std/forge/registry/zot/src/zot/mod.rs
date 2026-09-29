//! Internal Zot adapter modules.

mod client;
mod http;
mod model;
mod protocol;

#[cfg(test)]
mod tests;

pub use model::ZotHttpRegistry;
pub use model::{RegistryPullTokenProvider, ZotClientConfig, ZotClientError};
