//! Explicitly configured fresh current-manager continuation; no provider IO.
mod admission;
pub mod authority;
mod completed_drain;
pub mod context;
mod decoding;
mod original_closures;
mod terminal_history;

pub use context::selected;
