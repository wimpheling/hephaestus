use run_domain::{FreshRunExecutionProfile, RunState};
use run_orchestrator::RunRepository;
use run_postgres::{PgRunRepository, ProfiledRunRecoveryHoldReason};
use runtime_types::{LeaseId, VolumeId};
use sqlx::PgPool;

use super::{
    fixture::{self, Fixture},
    public_fixture::{Role, role_pool},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn missing_foreign_or_partial_tuple_and_late_vm_binding_roll_back(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, true).await;
    let lease = f.acquire().await.unwrap();
    let before = fixture::evidence(&pool, f.command.run_id).await;
    for (volume, id, fence) in [
        (None, None, None),
        (Some(lease.volume_id), None, Some(lease.fencing_token)),
        (
            Some(VolumeId::new()),
            Some(lease.id),
            Some(lease.fencing_token),
        ),
        (
            Some(lease.volume_id),
            Some(LeaseId::new()),
            Some(lease.fencing_token),
        ),
        (
            Some(lease.volume_id),
            Some(lease.id),
            Some(lease.fencing_token + 1),
        ),
    ] {
        assert!(
            f.runs
                .bind_resources(
                    f.command.run_id,
                    volume,
                    id,
                    fence,
                    &f.command.run_id.to_string()
                )
                .await
                .is_err()
        );
        assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    }
    assert!(
        f.runs
            .transition(f.command.run_id, RunState::Provisioning, None, None)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    f.bind(Some(&lease)).await;
    assert!(
        f.runs
            .bind_resources(
                f.command.run_id,
                Some(lease.volume_id),
                Some(lease.id),
                Some(lease.fencing_token),
                "foreign-vm"
            )
            .await
            .is_err()
    );
    f.runs
        .transition(f.command.run_id, RunState::Failed, None, Some("close"))
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    assert!(
        f.runs
            .bind_resources(
                f.command.run_id,
                Some(lease.volume_id),
                Some(lease.id),
                Some(lease.fencing_token),
                &f.command.run_id.to_string()
            )
            .await
            .is_err()
    );
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn revoked_released_pins_deny_new_io_but_not_cleanup(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, false).await;
    f.acquire().await;
    f.bind(None).await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(f.command.release_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = fixture::evidence(&pool, f.command.run_id).await;
    assert!(
        f.runs
            .transition(f.command.run_id, RunState::Provisioning, None, None)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    f.runs
        .transition(
            f.command.run_id,
            RunState::Failed,
            None,
            Some("withdrawn release"),
        )
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn cancellation_and_permanent_closure_deny_start_without_reopening(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, false).await;
    f.acquire().await;
    f.bind(None).await;
    let cancel = run_domain::CancelRun {
        command_id: runtime_types::CommandId::new(),
        run_id: f.command.run_id,
        reason: "controlled cancellation".into(),
    };
    assert!(f.runs.request_cancel(&cancel).await.unwrap());
    let before = fixture::evidence(&pool, f.command.run_id).await;
    assert!(
        f.runs
            .transition(f.command.run_id, RunState::Provisioning, None, None)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    f.runs
        .transition(f.command.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    let before = fixture::evidence(&pool, f.command.run_id).await;
    assert!(
        sqlx::query("UPDATE runs SET volume_acquisition_closed_at=NULL WHERE id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    assert!(
        f.runs
            .bind_resources(
                f.command.run_id,
                None,
                None,
                None,
                &f.command.run_id.to_string()
            )
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    f.runs
        .transition(f.command.run_id, RunState::CleanedUp, None, None)
        .await
        .unwrap();
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn raw_worker_io_and_terminal_bypass_and_actor_pool_are_denied(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, true).await;
    f.acquire().await;
    let before = fixture::evidence(&pool, f.command.run_id).await;
    assert!(
        sqlx::query("UPDATE runs SET state='provisioning' WHERE id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    let actor = role_pool(&pool, Role::Actor).await;
    let denied = PgRunRepository::new(actor.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
    assert!(
        denied
            .transition(f.command.run_id, RunState::Failed, None, None)
            .await
            .is_err()
    );
    assert!(denied.profiled_recovery_holds().await.is_err());
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    f.runs
        .transition(f.command.run_id, RunState::Failed, None, None)
        .await
        .unwrap();
    f.runs
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE runs SET state='cleaned_up' WHERE id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    actor.close().await;
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn explicit_recovery_reports_holds_and_never_promotes_history(pool: PgPool) {
    fixture::schema(&pool).await;
    let stateful = Fixture::new(&pool, true).await;
    let lease = stateful.acquire().await.unwrap();
    stateful.bind(Some(&lease)).await;
    let stateless = Fixture::new(&pool, false).await;
    stateless.acquire().await;
    stateless.bind(None).await;
    assert!(stateful.runs.recoverable_runs().await.unwrap().is_empty());
    let holds = stateful.runs.profiled_recovery_holds().await.unwrap();
    assert!(holds.iter().any(|h| h.run_id == stateful.command.run_id
        && h.reason == ProfiledRunRecoveryHoldReason::LegacyProviderPlacementUnproven));
    assert!(holds.iter().any(|h| h.run_id == stateless.command.run_id
        && h.reason == ProfiledRunRecoveryHoldReason::LegacyStatelessPlacementUnproven));
    let owned = stateful
        .runs
        .clone()
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(super::public_fixture::scope());
    assert!(
        owned
            .transition(stateful.command.run_id, RunState::Failed, None, None)
            .await
            .is_err()
    );
    assert!(
        owned
            .profiled_recovery_holds()
            .await
            .unwrap()
            .iter()
            .all(|h| h.reason == ProfiledRunRecoveryHoldReason::ProfileMismatch)
    );
    stateful.worker.close().await;
    stateless.worker.close().await;
}
