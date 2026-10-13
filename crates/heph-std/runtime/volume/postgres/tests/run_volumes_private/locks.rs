//! Real `PostgreSQL` row-lock ordering across consumers with opposite slot aliases.

use std::time::{Duration, Instant};

use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{RunVolumeMetadataRepository, VolumeError};

use super::named_fixture;
use crate::{fixture, support::Fixture};

pub async fn exercise(pool: &PgPool, seeded: &Fixture) {
    let (_, revision, _) = named_fixture::consumer(pool, seeded, 2).await;
    let volumes: Vec<Uuid> = sqlx::query_scalar(
        "SELECT volume_id FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=$1 ORDER BY volume_id",
    )
    .bind(revision)
    .fetch_all(pool)
    .await
    .expect("global resource order");
    let mut reversed = volumes.clone();
    reversed.reverse();
    let (instance_a, revision_a, agent_a) =
        named_fixture::reuse_consumer(pool, seeded, &reversed).await;
    let (instance_b, revision_b, agent_b) =
        named_fixture::reuse_consumer(pool, seeded, &volumes).await;
    let run_a = named_fixture::try_run(pool, seeded, instance_a, revision_a, agent_a)
        .await
        .expect("opposite slot aliases first run");
    let run_b = named_fixture::try_run(pool, seeded, instance_b, revision_b, agent_b)
        .await
        .expect("opposite slot aliases second run");
    let metadata = fixture::worker(pool).await;
    let set_a = metadata
        .load_run_selections(run_a)
        .await
        .expect("first exact set");
    let set_b = metadata
        .load_run_selections(run_b)
        .await
        .expect("second exact set");
    assert_eq!(
        set_a.selections()[0].scope().volume_id().as_uuid(),
        volumes[1]
    );
    assert_eq!(
        set_b.selections()[0].scope().volume_id().as_uuid(),
        volumes[0]
    );
    let mut blocker = pool
        .begin()
        .await
        .expect("higher resource lock transaction");
    sqlx::query("SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(volumes[1])
        .fetch_one(&mut *blocker)
        .await
        .expect("hold higher global resource");
    let (now, expiry) = fixture::times();
    let first = {
        let metadata = metadata.clone();
        tokio::spawn(async move { metadata.acquire_run(&set_a, "host", now, expiry).await })
    };
    assert_lower_locked(pool, volumes[0]).await;
    let second = {
        let metadata = metadata.clone();
        tokio::spawn(async move { metadata.acquire_run(&set_b, "host", now, expiry).await })
    };
    blocker.commit().await.expect("release higher resource");
    let first = tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("global order cannot deadlock")
        .expect("first task")
        .expect("first complete set");
    assert_eq!(first.len(), 2);
    let second = tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .expect("loser does not deadlock")
        .expect("second task");
    assert!(matches!(second, Err(VolumeError::LeaseConflict { .. })));
    assert_eq!(fixture::count(pool, run_a).await, 2);
    assert_eq!(
        fixture::count(pool, run_b).await,
        0,
        "conflict rolls back every lease"
    );
}

async fn assert_lower_locked(pool: &PgPool, volume: Uuid) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let probe = sqlx::query(
            "SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE NOWAIT",
        )
        .bind(volume)
        .fetch_one(pool)
        .await;
        if let Err(error) = probe {
            assert_eq!(
                error
                    .as_database_error()
                    .and_then(sqlx::error::DatabaseError::code)
                    .as_deref(),
                Some("55P03")
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "acquire must lock lower VolumeId before blocked higher slot"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
