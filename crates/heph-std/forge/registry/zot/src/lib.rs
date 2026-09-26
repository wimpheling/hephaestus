//! Authenticated, bounded Zot reads for registry reconciliation.
//!
//! The adapter addresses only an administrator-configured private Zot origin,
//! refuses redirects, requests exact digests, and validates returned bytes
//! before constructing control-plane evidence.

mod zot;

pub use zot::{RegistryPullTokenProvider, ZotClientConfig, ZotClientError, ZotHttpRegistry};
