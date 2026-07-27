//! Opt-in `PostgreSQL` integration coverage for the local volume store.

use runtime_types::{AgentId, RunId};
use sqlx::postgres::PgPoolOptions;
use std::{env, path::PathBuf, time::Duration};
use tempfile::TempDir;
use time::OffsetDateTime;
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_trait::{VolumeError, VolumeState, VolumeStore};

#[tokio::test]
async fn creates_formats_leases_rejects_and_recovers() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect to Postgres integration database");
    let temp = TempDir::new().expect("temporary volume root");
    let root = temp.path().canonicalize().expect("canonical volume root");
    let store = LocalVolumeStore::new(
        pool.clone(),
        LocalVolumeConfig {
            volume_root: root,
            transient_runtime_roots: Vec::new(),
            host_id: String::from("integration-host"),
            lease_duration: Duration::from_millis(2),
            mkfs_ext4: PathBuf::from("/usr/bin/mkfs.ext4"),
        },
    )
    .expect("local volume configuration");
    store.initialize().await.expect("runtime migrations");

    let agent_id = AgentId::new();
    let first = store
        .resolve_agent_state(agent_id, 32 * 1024 * 1024)
        .await
        .expect("create state volume");
    assert_eq!(first.state, VolumeState::Ready);
    assert!(first.host_path.is_file());
    let resolved = store
        .resolve_agent_state(agent_id, 64 * 1024 * 1024)
        .await
        .expect("resolve existing state volume");
    assert_eq!(resolved.id, first.id);
    assert_eq!(resolved.capacity_bytes, first.capacity_bytes);

    let first_run = RunId::new();
    let attachment = store
        .acquire(first.id, first_run)
        .await
        .expect("acquire first lease");
    let duplicate = store
        .acquire(first.id, first_run)
        .await
        .expect("same run acquires idempotently");
    assert_eq!(duplicate.lease.id, attachment.lease.id);
    let conflicting_run = RunId::new();
    assert!(matches!(
        store.acquire(first.id, conflicting_run).await,
        Err(VolumeError::LeaseConflict { .. })
    ));
    let attached = store
        .mark_attached(&attachment.lease)
        .await
        .expect("confirm attachment");
    store
        .release_after_detach(&attached)
        .await
        .expect("release after detach");
    assert!(first.host_path.is_file(), "release removed backing file");

    let stale = store
        .acquire(first.id, conflicting_run)
        .await
        .expect("acquire recovery lease");
    tokio::time::sleep(Duration::from_millis(5)).await;
    let stale_leases = store
        .stale_leases(OffsetDateTime::now_utc())
        .await
        .expect("find stale lease");
    assert_eq!(stale_leases, vec![stale.lease.clone()]);
    store
        .begin_recovery(&stale.lease)
        .await
        .expect("fence stale lease");
    assert!(matches!(
        store.release_after_detach(&stale.lease).await,
        Err(VolumeError::StaleLease)
    ));
    store
        .finish_recovery(&stale.lease)
        .await
        .expect("finish supervised recovery");
    store
        .acquire(first.id, RunId::new())
        .await
        .expect("volume reusable after recovery");

    sqlx::query("DELETE FROM volume_leases WHERE volume_id = $1")
        .bind(first.id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean lease fixtures");
    sqlx::query("DELETE FROM agent_state_volumes WHERE id = $1")
        .bind(first.id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean volume fixture");
    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(agent_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean agent fixture");
}
