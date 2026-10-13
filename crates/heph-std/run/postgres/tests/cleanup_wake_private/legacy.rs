//! Exact helper-only Legacy births; no inserted positive cleanup history.
#[path = "../profiled_lifecycle/fixture.rs"]
#[allow(dead_code)] // Reuse the genuine scalar producer; stateful synthetic fixture is unused.
mod base;
#[path = "../legacy_vm_placement/fixture.rs"]
#[allow(dead_code)] // Reuse checked old placement producer; raw close helper is unused.
mod fixture;
#[path = "../cleanup/fixture.rs"]
#[allow(dead_code)] // Reuse public checksum and role pools; cleanup row fabrication is unused.
mod public_fixture;
#[path = "../planned_creation/seed.rs"]
#[allow(dead_code)] // Required lexical helper dependency; update fixture is unused.
mod seed;
#[path = "../postgres/support.rs"]
#[allow(dead_code)] // Exact producer/request provenance; unrelated retry helpers are unused.
mod support;

use run_orchestrator::{
    RepositoryError, RunCleanupWakeOriginal, RunCleanupWakeRequest, RunRepository,
};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL gate"]
async fn genuine_consumed_legacy_placement_never_promotes_to_named_wake(pool: PgPool) {
    fixture::schema(&pool).await;
    let scope = fixture::scope();
    let f = fixture::fresh(&pool, false, &scope).await;
    let placement = f
        .runs
        .create_run_with_legacy_placement(&f.command, &scope)
        .await
        .unwrap();
    assert_eq!(placement.run.id, f.command.run_id);
    let plan = f
        .runs
        .assert_legacy_vm_placement_open(f.command.run_id, &scope)
        .await
        .unwrap();
    assert_eq!(
        plan.consumption(),
        Some(run_domain::LegacyVmPlacementConsumption::Provision)
    );
    let owner = run_domain::RunCleanupHostId::new(
        scope.provider_scope().namespace().into(),
        scope.provider_scope().host_id().into(),
    )
    .unwrap();
    let request = RunCleanupWakeRequest::new(
        f.command.run_id,
        f.command.instance_id,
        RunCleanupWakeOriginal {
            drain_attempt: Uuid::new_v4(),
            drain_command: Uuid::new_v4(),
            removal_id: Uuid::new_v4(),
            command_key: [5; 32],
            input_hash: [6; 32],
            expected_version: 1,
            creator: Uuid::new_v4(),
            request: Uuid::new_v4(),
        },
    )
    .unwrap();
    let repository = f.runs.clone().with_qualified_cleanup_wake(owner.clone());
    let before = base::evidence(&pool, f.command.run_id).await;
    for consumed in [false, true] {
        if consumed {
            f.runs
                .consume_legacy_vm_placement(
                    &f.command,
                    &scope,
                    run_domain::LegacyVmPlacementConsumption::Provision,
                )
                .await
                .unwrap();
        }
        let stable = base::evidence(&pool, f.command.run_id).await;
        assert!(matches!(
            repository.load_cleanup_wake(&request, &owner).await,
            Err(RepositoryError::InvalidData(
                "owning cleanup wake is unsupported for this composition"
            ))
        ));
        assert!(matches!(
            repository.prepare_cleanup_wake(&request, &owner).await,
            Err(RepositoryError::InvalidData(
                "owning cleanup wake is unsupported for this composition"
            ))
        ));
        assert_eq!(base::evidence(&pool, f.command.run_id).await, stable);
    }
    assert_eq!(
        base::evidence(&pool, f.command.run_id).await,
        before,
        "idempotent consumption never promotes history"
    );
}
