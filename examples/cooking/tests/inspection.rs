//! Authenticated HTTP inspection of the real mailbox cooking run.

// Keep the owner and outsider identities explicit at this boundary so each
// negative probe uses a real, independently seeded browser session.
#[path = "inspection/approval.rs"]
mod approval;
#[path = "inspection/diagnostics.rs"]
mod diagnostics;
#[path = "inspection/entry.rs"]
mod entry;
#[path = "inspection/rpc.rs"]
mod rpc;
#[path = "inspection/source.rs"]
mod source;
#[cfg(test)]
#[path = "inspection/tests.rs"]
mod tests;

pub use approval::approve_for_test;
pub(crate) use approval::{approve, assertion};
pub(crate) use diagnostics::emit_publication_diagnostic;
pub use entry::{inspect, inspect_with_https_uses};
pub(crate) use rpc::{get_json, rpc_failure_marker};
pub(crate) use source::{inspect_https, inspect_source};
