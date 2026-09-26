//! Authoritative reconciliation for forge-owned OCI registry content.
//!
//! Zot event notifications are merely lossy observations: this crate never
//! treats a callback as an approval signal. It reads Zot again by the exact
//! immutable digest, reduces that observation into typed actions, and leaves
//! lifecycle mutation to a separately authorized executor.

mod model;
mod ports;
mod reducer;
mod service;

pub use model::{
    AuthoritativeReconciliation, ClaimedNotification, Inconsistency, IntentReconciliation,
    NotificationCompletion, NotificationReduction, ObservedTarget, ReconciliationAction,
    ReconciliationPortError, ZotInspection,
};
pub use ports::{NotificationInbox, PublicationIntents, ReconciliationActionExecutor, ZotRegistry};
pub use service::RegistryReconciler;

#[cfg(test)]
mod tests {
    #[path = "authoritative.rs"]
    mod authoritative;
    #[path = "notifications.rs"]
    mod notifications;
    #[path = "support.rs"]
    mod support;
}
