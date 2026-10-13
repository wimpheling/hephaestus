use runtime_authority::{LiveRunSnapshotPins, LiveRunSnapshotRequest};
use sqlx::PgPool;
use uuid::Uuid;

use super::{fixture, producers, sessions};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn exact_pins_and_unproven_historical_producers_cannot_substitute(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    for index in 0..6 {
        let mut ids = [e.run, e.instance, e.revision, e.release, e.agent, e.command];
        ids[index] = Uuid::new_v4();
        let pins =
            LiveRunSnapshotPins::new(ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]).unwrap();
        fixture::denied(&repository, &LiveRunSnapshotRequest::pre_issuance(pins)).await;
    }
    let mut historical = e.clone();
    historical.run = f.run;
    historical.command = sqlx::query_scalar("SELECT command_id FROM runs WHERE id=$1")
        .bind(f.run)
        .fetch_one(&pool)
        .await
        .unwrap();
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&historical)),
    )
    .await;
    // A valid owned producer with no durable authenticated actor remains unsupported.
    let missing = super::fixture::Execution::normal(&pool, &f).await;
    producers::normal(&pool, &missing, "owned_canonical", true)
        .await
        .unwrap();
    fixture::advance(&pool, missing.run, "provisioning").await;
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&missing)),
    )
    .await;
    current_release_and_revision_must_still_match(&pool, &f, &e, &repository).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn legacy_and_accepted_mailbox_rows_are_not_owned_source_proof(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "legacy_scalar").await;
    let repository = fixture::repository(&pool).await;
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e)),
    )
    .await;
    let mailbox = fixture::Execution::normal(&pool, &f).await;
    let mut tx = fixture::worker(&pool).await;
    fixture::insert(&mut tx, &mailbox, "legacy_scalar", false, false)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE NONE")
        .execute(&mut *tx)
        .await
        .unwrap();
    producers::mailbox(&mut tx, &mailbox, &f).await;
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    fixture::advance(&pool, mailbox.run, "provisioning").await;
    let accepted: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE run_id=$1")
            .bind(mailbox.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(accepted, 1);
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&mailbox)),
    )
    .await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn acquisition_closure_and_cancellation_deny_preissuance(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    sessions::snapshot(&repository, &e).await;
    fixture::close(&pool, e.run).await;
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&e)),
    )
    .await;
    let cancelled = fixture::normal(&pool, &f, "owned_canonical").await;
    let mut tx = fixture::worker(&pool).await;
    sqlx::query("UPDATE runs SET cancel_requested_at=now() WHERE id=$1")
        .bind(cancelled.run)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    fixture::denied(
        &repository,
        &LiveRunSnapshotRequest::pre_issuance(fixture::pins(&cancelled)),
    )
    .await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn permanent_removal_denies_existing_live_session(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    fixture::advance(&pool, e.run, "provisioning").await;
    let session = sessions::issue(&repository, &e, &snapshot).await;
    let mut tx = pool.begin().await.unwrap();
    let request = Uuid::new_v4();
    sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.request_id',$2,true)")
        .bind(f.maintainer.to_string()).bind(request.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO agent_instance_removal_requests(id,instance_id,expected_version,input_hash,created_by,request_id,authorization_model_version) SELECT $1,id,version,$3,$4,$5,'live-fixture/v1' FROM agent_instances WHERE id=$2")
        .bind(Uuid::new_v4()).bind(e.instance).bind([12_u8;32].as_slice()).bind(f.maintainer.as_uuid()).bind(request).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    fixture::denied(&repository, &sessions::request(&e, &session, &snapshot)).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private profile loader"]
async fn original_source_role_withdrawal_and_user_suspension_deny_live_checks(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::normal(&pool, &f, "owned_canonical").await;
    let repository = fixture::repository(&pool).await;
    let snapshot = sessions::snapshot(&repository, &e).await;
    fixture::advance(&pool, e.run, "provisioning").await;
    let issued = sessions::issue(&repository, &e, &snapshot).await;
    let request = sessions::request(&e, &issued, &snapshot);
    fixture::advance(&pool, e.run, "running").await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.project)
        .bind(f.maintainer.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    fixture::denied(&repository, &request).await;
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(f.project)
        .bind(f.maintainer.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sessions::allowed(&repository, &request, &snapshot).await;
    for status in ["suspended", "disabled"] {
        sqlx::query("UPDATE users SET status=$2 WHERE id=$1")
            .bind(f.maintainer.as_uuid())
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        let membership: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM project_maintainers WHERE project_id=$1 AND user_id=$2)",
        )
        .bind(f.project)
        .bind(f.maintainer.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            membership,
            "suspension must deny even with source membership retained"
        );
        fixture::denied(&repository, &request).await;
    }
    sqlx::query("UPDATE users SET status='active' WHERE id=$1")
        .bind(f.maintainer.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sessions::allowed(&repository, &request, &snapshot).await;
}

async fn current_release_and_revision_must_still_match(
    pool: &PgPool,
    f: &crate::support::Fixture,
    e: &fixture::Execution,
    repository: &runtime_authority_postgres::PgRuntimeSessionRepository,
) {
    let snapshot = sessions::snapshot(repository, e).await;
    let request = LiveRunSnapshotRequest::pre_issuance(fixture::pins(e));
    let candidate: Uuid =
        sqlx::query_scalar("SELECT candidate_revision_id FROM agent_updates WHERE id=$1")
            .bind(f.update)
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(e.instance)
        .bind(candidate)
        .execute(pool)
        .await
        .unwrap();
    fixture::denied(repository, &request).await;
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(e.instance)
        .bind(e.revision)
        .execute(pool)
        .await
        .unwrap();
    sessions::allowed(repository, &request, &snapshot).await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(e.release)
        .execute(pool)
        .await
        .unwrap();
    fixture::denied(repository, &request).await;
}
