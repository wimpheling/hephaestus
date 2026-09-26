//! Domain primitives for one-time UI handoffs and generation-bound browser
//! sessions.
//!
//! This module contains no persistence or transport behavior. Raw bearer
//! secrets are intentionally neither serializable nor displayable; adapters
//! hash them before storage and place them only at their explicit transport
//! boundaries.

#[path = "ui_browser/identifiers.rs"]
mod identifiers;
#[path = "ui_browser/routes.rs"]
mod routes;
#[path = "ui_browser/secrets.rs"]
mod secrets;
#[path = "ui_browser/session.rs"]
mod session;

pub use identifiers::{UiBrowserHandoffId, UiBrowserSessionId};
pub use routes::{UiBrowserRoute, UiBrowserRouteError};
pub use secrets::{
    UiBrowserHandoffDigest, UiBrowserHandoffSecret, UiBrowserSessionDigest, UiBrowserSessionSecret,
};
pub use session::{UiBrowserHandoffFailure, UiBrowserSessionFailure, child_session_expiry};

/// Lifetime of a one-time browser handoff.
pub const UI_BROWSER_HANDOFF_TTL_SECONDS: i64 = 60;
/// Maximum lifetime of a child UI browser session from its issue instant.
pub const UI_BROWSER_SESSION_TTL_SECONDS: i64 = 12 * 60 * 60;

#[cfg(test)]
#[path = "ui_browser/tests.rs"]
mod tests;
