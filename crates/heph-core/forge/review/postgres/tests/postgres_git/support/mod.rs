//! Shared `PostgreSQL` and Git fixtures for review integration tests.

#[path = "control.rs"]
mod control;
#[path = "fixture.rs"]
mod fixture;
#[path = "git.rs"]
mod git;

pub use control::insert_control;
pub use fixture::seed;
pub use git::{git_text, git_text_with_identity, pool, run_git};
