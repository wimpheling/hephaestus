//! Genuine never-consumed Invocation histories admit fresh managed Remove recovery.
//! The frozen fixture provisions real ext4; these cases launch no VM or guest.

#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/git.rs"]
mod git;
#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/fixture.rs"]
#[allow(dead_code)] // Reuse the original producer without copying its unrelated routing checks.
mod original;
#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/parents.rs"]
mod parents;
mod schema {
    pub use crate::fixture::{install, role_pool};
}
#[path = "ordinary_queued_cancel/assertions.rs"]
#[allow(dead_code)] // This target reuses the real receipt/authority checks, not all denial cases.
mod assertions;
#[path = "queued_remove_resume/closure_guard.rs"]
mod closure_guard;
#[path = "ordinary_queued_cancel/fixture.rs"]
mod fixture;
#[path = "ordinary_queued_cancel/instance.rs"]
mod instance;
#[path = "ordinary_queued_cancel/queued.rs"]
mod queued;
#[path = "ordinary_queued_cancel/volume.rs"]
mod volume;
mod queued_remove_resume {
    pub mod denial;
    pub mod scenario;
}

use std::sync::Arc;

async fn owning(pool: sqlx::PgPool, empty: bool) {
    let fixture = Arc::new(fixture::Fixture::new(pool).await);
    tokio::spawn(async move {
        use futures_util::FutureExt;
        let result = std::panic::AssertUnwindSafe(async {
            queued_remove_resume::scenario::run(&fixture, empty).await;
            fixture.close().await;
        })
        .catch_unwind()
        .await;
        if let Err(failure) = result {
            // Retain original provider/Volume owner/config/three pools on an
            // uncertain test outcome. Dropping the waiter proves no cleanup.
            std::mem::forget(fixture);
            std::panic::resume_unwind(failure);
        }
    })
    .await
    .expect("original-custody queued Remove scenario");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable head141 PostgreSQL, local Git, delegated cgroup and actual ext4/libkrun assets"]
async fn fresh_manager_preserves_completed_ordinary_and_pending_queued_drain(pool: sqlx::PgPool) {
    owning(pool, false).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable head141 PostgreSQL, local Git, delegated cgroup and actual ext4/libkrun assets"]
async fn empty_prepared_instance_has_positive_resume_and_orphan_denial(pool: sqlx::PgPool) {
    owning(pool, true).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable head142 PostgreSQL, local Git, delegated cgroup and actual ext4/libkrun assets"]
async fn app_closure_stamp_qualifies_live_ordinary_control_and_denies_late_replay(
    pool: sqlx::PgPool,
) {
    let fixture = Arc::new(fixture::Fixture::new(pool).await);
    tokio::spawn(async move {
        use futures_util::FutureExt;
        let result = std::panic::AssertUnwindSafe(async {
            Box::pin(closure_guard::run(&fixture)).await;
            fixture.close().await;
        })
        .catch_unwind()
        .await;
        if let Err(failure) = result {
            // A failed native guard observation does not release original custody.
            std::mem::forget(fixture);
            std::panic::resume_unwind(failure);
        }
    })
    .await
    .expect("original-custody mixed app closure guard scenario");
}
