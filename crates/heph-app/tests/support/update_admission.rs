//! Durable SQL fixtures and assertions for update admission regressions.
//!
//! Generated request construction remains in `support::rpc::update_admission`.

#[path = "update_admission/fixture.rs"]
mod fixture;

pub use fixture::exercise;
#[cfg(feature = "test-fixtures")]
pub use fixture::exercise_reconciler_wins_race;
