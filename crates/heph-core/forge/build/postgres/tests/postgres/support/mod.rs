//! Shared fixtures for the isolated build integration.

#[path = "config.rs"]
mod config;
#[path = "fixture.rs"]
mod fixture;
#[path = "git.rs"]
mod git;
#[path = "vm.rs"]
mod vm;

pub use config::CONFIG;
pub use fixture::{copy_build_request, seed};
pub use git::source_repository;
pub use vm::OutputProvider;
