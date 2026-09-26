//! Checked-in protobuf messages and Connect service/client contracts.
//!
//! This crate contains generated transport types only. Domain and application
//! crates must define inward-facing models and convert at the RPC boundary.

// This crate is checked-in machine output plus a minimal reflection facade.
// Generated bindings cannot be made to follow repository style lints by hand.
#![allow(
    clippy::all,
    clippy::nursery,
    clippy::pedantic,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::too_many_lines,
    missing_docs
)]

/// Generated protobuf message modules.
#[path = "generated/messages/mod.rs"]
pub mod messages;

/// Generated Connect service traits and clients.
#[path = "generated/connect/mod.rs"]
pub mod connect;

/// Complete descriptor set, including imports, for server reflection and
/// descriptor-policy checks.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("generated/descriptor.binpb");

/// Decodes the checked-in descriptor set for `connectrpc-reflection`.
pub fn descriptor_pool() -> Result<buffa_descriptor::DescriptorPool, buffa_descriptor::PoolError> {
    buffa_descriptor::DescriptorPool::decode(FILE_DESCRIPTOR_SET)
}
