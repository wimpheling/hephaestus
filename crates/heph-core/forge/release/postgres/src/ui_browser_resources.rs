//! Application-role host and UI resource adapter.
//!
//! It is deliberately separate from worker handoff writes and contains no
//! listener or HTTP-cookie code.

use sqlx::PgPool;

mod host;
mod resources;
mod support;
mod target;

pub use host::PgUiGenerationHostResolver;

/// Application-role verifier plus current static/gateway declaration reader.
#[derive(Clone)]
pub struct PgUiBrowserServingStore {
    pub(super) app_pool: PgPool,
}

impl PgUiBrowserServingStore {
    /// Creates the serving projection over an application-role pool.
    #[must_use]
    pub const fn new(app_pool: PgPool) -> Self {
        Self { app_pool }
    }
}
