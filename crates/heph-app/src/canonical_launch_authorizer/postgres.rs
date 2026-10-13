//! Actual source/caller checks; trusted producer fixtures are not HTTP proof.

#[path = "postgres_guards.rs"]
mod guards;
#[path = "postgres_normal.rs"]
mod normal;
#[path = "postgres_update.rs"]
mod update;

use std::sync::Arc;

use authz_postgres::PostgresMelangeAuthorizer;
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use runtime_authority_postgres::PgRuntimeSessionRepository;

use super::CanonicalRunLaunchAuthorizer;
use crate::runtime_authority_manager::postgres::fixture;

fn scalar(pool: &sqlx::PgPool) -> Arc<PgRunLaunchAuthorizer> {
    Arc::new(PgRunLaunchAuthorizer::new(
        pool.clone(),
        Arc::new(PostgresMelangeAuthorizer),
    ))
}

fn wrapper(f: &fixture::Fixture) -> CanonicalRunLaunchAuthorizer {
    CanonicalRunLaunchAuthorizer::new(
        PgRuntimeSessionRepository::new(f.worker.clone()).with_canonical_live_snapshots(),
        scalar(&f.worker),
    )
}
