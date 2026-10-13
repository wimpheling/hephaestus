//! Genuine ordinary Cancel preserves a live instance's nonempty volume authority.
//! Native ext4 provisioning is required; no VM or guest is launched.

#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/git.rs"]
mod git;
#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/fixture.rs"]
#[allow(dead_code)] // The existing genuine Git producer also exposes unrelated routing checks.
mod original;
#[path = "../../heph-std/run/postgres/tests/qualified_ordinary_routing/parents.rs"]
mod parents;
mod schema {
    pub use crate::fixture::{install, role_pool};
}
#[path = "ordinary_queued_cancel/assertions.rs"]
mod assertions;
#[path = "ordinary_queued_cancel/fixture.rs"]
mod fixture;
#[path = "ordinary_queued_cancel/instance.rs"]
mod instance;
#[path = "ordinary_queued_cancel/queued.rs"]
mod queued;
#[path = "ordinary_queued_cancel/secret_cleanup.rs"]
mod secret_cleanup;
#[path = "ordinary_queued_cancel/volume.rs"]
mod volume;

use std::sync::Arc;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL head141, local Git, delegated cgroup and configured native ext4/libkrun assets"]
async fn ordinary_cancel_keeps_nonempty_grants_and_sibling_invocation(pool: sqlx::PgPool) {
    let fixture = Arc::new(fixture::Fixture::new(pool).await);
    // The original owners and three pools live in this task through all effects
    // AND pool closure. Aborting the waiter cannot cancel admitted work.
    tokio::spawn(async move {
        use futures_util::FutureExt;
        let result = std::panic::AssertUnwindSafe(async {
            scenario(&fixture).await;
            fixture.close().await;
        })
        .catch_unwind()
        .await;
        if let Err(failure) = result {
            // A failed assertion or unknown cleanup retains these ORIGINAL
            // owners even if the outer waiter was cancelled. This disposable
            // test leak proves neither cleanup nor recovery.
            std::mem::forget(fixture);
            std::panic::resume_unwind(failure);
        }
    })
    .await
    .expect("owning nonempty cancellation scenario");
}

async fn scenario(fixture: &fixture::Fixture) {
    let prepared = instance::prepare(fixture).await;
    let volume = volume::create(fixture, &prepared).await;
    instance::import(fixture, &prepared).await;
    let starts = queued::activate_and_invoke(fixture, &prepared).await;
    let before = assertions::Snapshot::capture(fixture, &prepared.instance, &starts, volume).await;
    before.require_nonempty();
    secret_cleanup::queued_denied(fixture, starts[0].run_id).await;
    let controller = fixture.controller().await;
    let (cancel, completion) =
        cancel_and_replay(fixture, &prepared, &starts, &before, &controller).await;
    secret_cleanup::completed(fixture, starts[0].run_id).await;
    let second = queued::cancel(
        fixture,
        &prepared.actor,
        starts[0].run_id,
        "second genuine user request",
    )
    .await;
    assert_ne!(cancel.command_id, second.command_id);
    assert!(controller.clone().cancel_run(&second).await.unwrap());
    assertions::processed(fixture, &second).await;
    assertions::cleaned(fixture, &controller, starts[0].run_id).await;
    assert_eq!(
        completion,
        assertions::completion(fixture, starts[0].run_id).await
    );
    before
        .assert_preserved(fixture, &prepared.instance, &starts)
        .await;
    assertions::denials(fixture, starts[0].run_id, &cancel).await;
    assert_eq!(
        completion,
        assertions::completion(fixture, starts[0].run_id).await
    );
    before
        .assert_preserved(fixture, &prepared.instance, &starts)
        .await;
    drop(controller);
}

async fn cancel_and_replay(
    fixture: &fixture::Fixture,
    prepared: &instance::Prepared,
    starts: &[run_domain::StartRun; 2],
    before: &assertions::Snapshot,
    controller: &Arc<hephaestus_app::ManagedInvocationController>,
) -> (run_domain::CancelRun, serde_json::Value) {
    let cancel = queued::cancel(
        fixture,
        &prepared.actor,
        starts[0].run_id,
        "ordinary user cancellation",
    )
    .await;
    assert!(controller.clone().cancel_run(&cancel).await.unwrap());
    assertions::processed(fixture, &cancel).await;
    assertions::cleaned(fixture, controller, starts[0].run_id).await;
    before
        .assert_preserved(fixture, &prepared.instance, starts)
        .await;
    let completion = assertions::completion(fixture, starts[0].run_id).await;
    assert!(!controller.clone().cancel_run(&cancel).await.unwrap());
    assert_eq!(
        completion,
        assertions::completion(fixture, starts[0].run_id).await
    );
    (cancel, completion)
}
