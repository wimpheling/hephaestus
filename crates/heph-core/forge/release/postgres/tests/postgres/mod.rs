//! Shared fixtures and helpers for the release `PostgreSQL` integration target.

use super::*;

#[path = "build.rs"]
/// Release build and publication fixtures.
pub mod build;
#[path = "gateway.rs"]
/// Gateway revision and installation fixtures.
pub mod gateway;
#[path = "locking.rs"]
/// Database lock coordination helpers.
pub mod locking;
#[path = "manifests.rs"]
/// Artifact and manifest fixtures.
pub mod manifests;
#[path = "seeding_core.rs"]
/// Base pool, identity, and fixture seeding helpers.
pub mod seeding_core;
#[path = "seeding_releases.rs"]
/// Release and update fixture helpers.
pub mod seeding_releases;

pub(crate) use build::*;
pub(crate) use gateway::*;
pub(crate) use locking::*;
pub(crate) use manifests::*;
pub(crate) use seeding_core::*;
pub(crate) use seeding_releases::*;
