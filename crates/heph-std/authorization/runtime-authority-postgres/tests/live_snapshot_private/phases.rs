use runtime_authority::{
    LiveRunSnapshotRequest, LiveRunSnapshotResolver, RuntimeAuthorityError,
    RuntimeSessionRepository,
};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::{fixture, sessions};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn preissuance_supports_lease_and_provision_without_widening_issuance(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    assert_eq!(snapshot.bindings().len(), 0);
    assert!(
        repository
            .resolve_snapshot(
                &fixture::run(&e, run_domain::RunState::LeasingVolume),
                "live-fixture/v1"
            )
            .await
            .is_err()
    );
    fixture::advance(&pool, e.run, "provisioning").await;
    assert_eq!(
        repository
            .resolve_snapshot(
                &fixture::run(&e, run_domain::RunState::Provisioning),
                "live-fixture/v1"
            )
            .await
            .unwrap(),
        snapshot
    );
    sessions::allowed(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e)),
        &snapshot,
    )
    .await;
    fixture::advance(&pool, e.run, "starting").await;
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e)),
    )
    .await;
    assert!(
        repository
            .resolve_snapshot(
                &fixture::run(&e, run_domain::RunState::Starting),
                "live-fixture/v1"
            )
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn default_and_nonworker_resolvers_deny_before_owned_profile_authority(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let request = LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e));
    let default = PgRuntimeSessionRepository::new(pool.clone());
    assert_eq!(
        default
            .resolve_live_snapshot(&request, "live-fixture/v1", OffsetDateTime::now_utc())
            .await
            .unwrap_err(),
        RuntimeAuthorityError::Unsupported
    );
    let admin = default.with_canonical_live_snapshots();
    assert_eq!(
        admin
            .resolve_live_snapshot(&request, "live-fixture/v1", OffsetDateTime::now_utc())
            .await
            .unwrap_err(),
        RuntimeAuthorityError::Unsupported
    );
    let actor = sqlx::postgres::PgPoolOptions::new()
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    fixture::denied(
        &PgRuntimeSessionRepository::new(actor).with_canonical_live_snapshots(),
        &request,
    )
    .await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn pending_to_active_and_owned_terminal_drain_keep_exact_issued_ceiling(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    fixture::advance(&pool, e.run, "provisioning").await;
    let session = sessions::issue(&repository, &e, &snapshot).await;
    let request = sessions::request(&e, &session, &snapshot);
    fixture::advance(&pool, e.run, "starting").await;
    sessions::allowed(&repository, &request, &snapshot).await;
    let active = repository
        .acknowledge(session.id, session.generation, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(
        active.status,
        capability_domain::RuntimeSessionStatus::Active
    );
    // The request intentionally retains the earlier pending status.
    for state in ["running", "succeeded", "cleaning_up"] {
        fixture::advance(&pool, e.run, state).await;
        sessions::allowed(&repository, &request, &snapshot).await;
    }
    fixture::close(&pool, e.run).await;
    sessions::allowed(&repository, &request, &snapshot).await;
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e)),
    )
    .await;
    assert!(
        repository
            .resolve_snapshot(
                &fixture::run(&e, run_domain::RunState::CleaningUp),
                "live-fixture/v1"
            )
            .await
            .is_err()
    );
    failed_drain_keeps_original_session(&pool, &f, &repository).await;
}

async fn failed_drain_keeps_original_session(
    pool: &PgPool,
    f: &crate::support::Fixture,
    repository: &PgRuntimeSessionRepository,
) {
    let e = fixture::normal(pool, f, "owned_canonical").await;
    let snapshot = sessions::snapshot(repository, &e).await;
    fixture::advance(pool, e.run, "provisioning").await;
    let session = sessions::issue(repository, &e, &snapshot).await;
    let request = sessions::request(&e, &session, &snapshot);
    fixture::advance(pool, e.run, "running").await;
    fixture::advance(pool, e.run, "failed").await;
    fixture::close(pool, e.run).await;
    sessions::allowed(repository, &request, &snapshot).await;
    fixture::advance(pool, e.run, "cleaning_up").await;
    sessions::allowed(repository, &request, &snapshot).await;
}
