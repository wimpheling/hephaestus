//! Redacted capability audit recording and authorized inspection contracts.
//!
//! This boundary deliberately accepts only opaque identifiers, closed enums,
//! and bounded machine reason codes. It cannot receive request payloads,
//! credentials, paths, secret values, or provider responses.

mod error;
mod events;
mod records;
mod vocabulary;

#[cfg(test)]
mod tests;

/// Maximum number of events returned by one inspection request.
pub const MAX_AUDIT_PAGE_SIZE: u16 = 200;

pub use error::CapabilityAuditError;
pub use events::{
    CapabilityAuditContext, CapabilityAuditCursor, CapabilityAuditPage, NewCapabilityAuditEvent,
};
pub use records::{CapabilityAuditRecord, CapabilityAuditRepository};
pub use vocabulary::{
    CapabilityAuditEventKind, CapabilityAuditReason, CapabilityDecision, CapabilityUseOutcome,
};
