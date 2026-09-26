//! Authorized release read operations used by the transport layer.
#![allow(clippy::unused_async)] // Query methods retain async transport contracts while adapter SQL is introduced.

pub mod ui;

mod application;
mod helpers;
mod rows;
mod types;

#[cfg(test)]
mod tests;

pub use application::ReleaseApplication;
pub use types::*;
