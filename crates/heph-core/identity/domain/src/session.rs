//! Durable browser-session identity values.
//!
//! The session ID is a bearer value at the browser and mediator boundaries.
//! Its `Debug` and `Display` implementations are deliberately redacted;
//! callers must use [`BrowserSessionSid::to_protocol_string`] only when
//! explicitly serializing the ID for those protocols.

mod digests;
mod identifiers;
mod metadata;

#[cfg(test)]
mod tests;

/// Default server-selected browser-session lifetime.
pub const DEFAULT_BROWSER_SESSION_TTL_SECONDS: i64 = 12 * 60 * 60;
/// Database-enforced upper bound for a browser-session lifetime.
pub const MAX_BROWSER_SESSION_TTL_SECONDS: i64 = 24 * 60 * 60;

pub use digests::{browser_session_identity_binding_digest, browser_session_sid_digest};
pub use identifiers::{
    BrowserSessionDigest, BrowserSessionId, BrowserSessionIdentityBindingDigest, BrowserSessionSid,
};
pub use metadata::{BrowserSessionMetadata, BrowserSessionRevocationReason};
