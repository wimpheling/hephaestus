use runtime_types::{RunId, VolumeId};
use sqlx::PgPool;
use volume_trait::VolumeMetadataRepository;

use super::{fixture, producers};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn stateless_positive_legacy_empty_projection_is_lifecycle_only(pool: PgPool) {
    let f = fixture::setup(&pool).await;
    let e = fixture::Execution::stateless(&pool, &f).await;
    producers::normal(&pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    let repository = fixture::repository(&pool).await;
    assert!(
        repository
            .active_lease_for_run(RunId::from_uuid(e.run), "host")
            .await
            .unwrap()
            .is_none()
    );
    let before = fixture::evidence(&pool, f.volume).await;
    let (now, expires) = fixture::times();
    assert!(
        repository
            .acquire(
                VolumeId::from_uuid(f.volume),
                RunId::from_uuid(e.run),
                "host",
                now,
                expires
            )
            .await
            .is_err()
    );
    fixture::advance(&pool, e.run, "cleaning_up").await;
    fixture::advance(&pool, e.run, "cleaned_up").await;
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
    let canonical: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_cleanup_targets WHERE run_id=$1")
            .bind(e.run)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(canonical, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn qualified_legacy_mailbox_and_update_use_same_scalar_origin_without_delegation(
    pool: PgPool,
) {
    let f = fixture::setup(&pool).await;
    let mailbox = fixture::Execution::normal(&pool, &f).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    fixture::insert(&mut tx, &mailbox, "legacy_scalar", false, false)
        .await
        .unwrap();
    // The attempt's immediate Run FK requires the parent first; the exact
    // producer/profile proof is checked against both rows at transaction commit.
    sqlx::query("SET LOCAL ROLE NONE")
        .execute(&mut *tx)
        .await
        .unwrap();
    producers::mailbox(&mut tx, &mailbox, &f).await;
    tx.commit().await.unwrap();
    fixture::advance(&pool, mailbox.run, "leasing_volume").await;
    let repository = fixture::repository(&pool).await;
    let (now, expires) = fixture::times();
    let lease = repository
        .acquire(
            VolumeId::from_uuid(f.volume),
            RunId::from_uuid(mailbox.run),
            "host",
            now,
            expires,
        )
        .await
        .unwrap();
    fixture::advance(&pool, mailbox.run, "cleaning_up").await;
    repository
        .release_after_detach(&lease, false)
        .await
        .unwrap();
    fixture::advance(&pool, mailbox.run, "cleaned_up").await;
    let (update, result) = producers::update(&pool, &f, false).await;
    result.unwrap();
    fixture::advance(&pool, update.run, "leasing_volume").await;
    let (now, expires) = fixture::times();
    let next = repository
        .acquire(
            lease.volume_id,
            RunId::from_uuid(update.run),
            "host",
            now,
            expires,
        )
        .await
        .unwrap();
    assert_eq!(next.fencing_token, lease.fencing_token + 1);
    let row = fixture::evidence(&pool, f.volume).await;
    let selected = row["leases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == next.id.to_string())
        .unwrap();
    assert_eq!(
        selected["selected_revision_id"],
        update.revision.to_string()
    );
    assert!(
        selected["update_id"].is_null(),
        "hook lease uses exact admitted Run, not historical update-only identity"
    );
    fixture::advance(&pool, update.run, "cleaning_up").await;
    repository.release_after_detach(&next, false).await.unwrap();
    fixture::advance(&pool, update.run, "cleaned_up").await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private scalar PostgreSQL"]
async fn released_tuple_read_helper_requires_cleaned_lifecycle_but_not_current_fence(pool: PgPool) {
    let (f, e, repository, lease) = fixture::acquired(&pool).await;
    fixture::advance(&pool, e.run, "cleaning_up").await;
    repository
        .release_after_detach(&lease, false)
        .await
        .unwrap();
    let mut tx = fixture::worker(&pool).await;
    let before: bool =
        sqlx::query_scalar("SELECT legacy_scalar_lease_was_released($1,$2,$3,$4,$5)")
            .bind(lease.id.as_uuid())
            .bind(e.run)
            .bind(f.volume)
            .bind("host")
            .bind(lease.fencing_token)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(!before);
    tx.rollback().await.unwrap();
    fixture::advance(&pool, e.run, "cleaned_up").await;
    let next = fixture::Execution::normal(&pool, &f).await;
    producers::normal(&pool, &next, "legacy_scalar", false)
        .await
        .unwrap();
    fixture::advance(&pool, next.run, "leasing_volume").await;
    let (now, expires) = fixture::times();
    repository
        .acquire(
            lease.volume_id,
            RunId::from_uuid(next.run),
            "host",
            now,
            expires,
        )
        .await
        .unwrap();
    let before = fixture::evidence(&pool, f.volume).await;
    let mut tx = fixture::worker(&pool).await;
    let complete: bool =
        sqlx::query_scalar("SELECT legacy_scalar_lease_was_released($1,$2,$3,$4,$5)")
            .bind(lease.id.as_uuid())
            .bind(e.run)
            .bind(f.volume)
            .bind("host")
            .bind(lease.fencing_token)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let wrong: bool = sqlx::query_scalar("SELECT legacy_scalar_lease_was_released($1,$2,$3,$4,$5)")
        .bind(lease.id.as_uuid())
        .bind(e.run)
        .bind(f.volume)
        .bind("host")
        .bind(lease.fencing_token + 1)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(complete);
    assert!(!wrong);
    tx.commit().await.unwrap();
    assert_eq!(fixture::evidence(&pool, f.volume).await, before);
}
