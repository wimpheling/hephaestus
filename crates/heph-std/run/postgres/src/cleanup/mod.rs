//! Trusted worker persistence for canonical run cleanup.

mod finish;
mod invocation_recovery;
pub mod model;
mod queued;
mod receipts;
mod repository;
mod targets;

pub use repository::PgRunCleanupRepository;
