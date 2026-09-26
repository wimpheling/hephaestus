//! Authenticated, bounded Zot event notification ingestion.
//!
//! Zot v2.1.18 sends registry events as binary-mode `CloudEvents` over HTTP.
//! This crate validates that transport contract and returns only metadata
//! suitable for a durable inbox. It intentionally owns neither publication
//! approval nor product-event emission: a callback only requests reconciliation.

mod credential;
mod model;
mod parser;
mod path;
mod timestamp;

pub use credential::CallbackCredential;
pub use model::{
    NotificationAction, NotificationIdempotencyKey, NotificationObservation,
    ObservedRepositoryPath, PayloadSha256, ZotEventType,
};
pub use parser::NotificationError;
pub use parser::parse_notification;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
