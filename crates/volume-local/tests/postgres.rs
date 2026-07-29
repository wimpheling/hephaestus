//! Opt-in `PostgreSQL` integration coverage for the local volume store.

use runtime_types::{AgentInstanceId, RunId};
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

    let instance_id = AgentInstanceId::new();
    let fixture = seed_instance(&pool, instance_id).await;
    let first = store
        .resolve_instance_state(instance_id, 32 * 1024 * 1024)
        .await
        .expect("create state volume");
    assert_eq!(first.state, VolumeState::Ready);
    assert!(first.host_path.is_file());
    let resolved = store
        .resolve_instance_state(instance_id, 64 * 1024 * 1024)
        .await
        .expect("resolve existing state volume");
    assert_eq!(resolved.id, first.id);
    assert_eq!(resolved.capacity_bytes, first.capacity_bytes);

    let first_run = RunId::new();
    seed_run(&pool, &fixture, first_run).await;
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
    seed_run(&pool, &fixture, conflicting_run).await;
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
    let recovered_run = RunId::new();
    seed_run(&pool, &fixture, recovered_run).await;
    store
        .acquire(first.id, recovered_run)
        .await
        .expect("volume reusable after recovery");

    cleanup(&pool, instance_id, first.id).await;
}

// Exact provenance is clearer here with the schema's canonical identifier names.
#[allow(clippy::struct_field_names)]
struct InstanceFixture {
    instance_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    release_id: uuid::Uuid,
    release_agent_id: uuid::Uuid,
    attachment_id: uuid::Uuid,
}

// Keeping the complete foreign-key graph together makes this persistence fixture auditable.
#[allow(clippy::too_many_lines)]
async fn seed_instance(pool: &sqlx::PgPool, instance_id: AgentInstanceId) -> InstanceFixture {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    let repository_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let build_id = uuid::Uuid::new_v4();
    let release_id = uuid::Uuid::new_v4();
    let release_agent_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let attachment_id = uuid::Uuid::new_v4();
    let volume_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("volume-{organization_id}"))
        .execute(pool)
        .await
        .expect("volume organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("volume-{project_id}"))
        .execute(pool)
        .await
        .expect("volume project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository_id)
        .bind(project_id)
        .bind(format!("volume-{repository_id}"))
        .execute(pool)
        .await
        .expect("volume repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'volume')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("volume family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("volume build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind(format!("volume-{release_id}"))
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("volume release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'volume', 'Volume', '{}', $4, true)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("volume release agent");
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id.as_uuid())
    .bind(project_id)
    .bind(family_id)
    .bind(format!("volume-{instance_id}"))
    .execute(pool)
    .await
    .expect("volume instance");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
         (id, instance_id, state, capacity_bytes)
         VALUES ($1, $2, 'uninitialized', 33554432)",
    )
    .bind(volume_id)
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("instance state volume");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)",
    )
    .bind(revision_id)
    .bind(instance_id.as_uuid())
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("volume revision");
    sqlx::query(
        "UPDATE agent_instances
         SET active_revision_id = $2, state_volume_id = $3 WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .bind(revision_id)
    .bind(volume_id)
    .execute(pool)
    .await
    .expect("activate volume instance");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')",
    )
    .bind(attachment_id)
    .bind(instance_id.as_uuid())
    .bind(project_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("volume attachment");
    InstanceFixture {
        instance_id: instance_id.as_uuid(),
        revision_id,
        release_id,
        release_agent_id,
        attachment_id,
    }
}

async fn seed_run(pool: &sqlx::PgPool, fixture: &InstanceFixture, run_id: RunId) {
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, command_id, state, requires_state,
          created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'normal', $7, 'queued', true, now(), now())",
    )
    .bind(run_id.as_uuid())
    .bind(fixture.instance_id)
    .bind(fixture.revision_id)
    .bind(fixture.release_id)
    .bind(fixture.release_agent_id)
    .bind(fixture.attachment_id)
    .bind(uuid::Uuid::new_v4())
    .execute(pool)
    .await
    .expect("volume run");
}

async fn cleanup(
    pool: &sqlx::PgPool,
    instance_id: AgentInstanceId,
    volume_id: runtime_types::VolumeId,
) {
    sqlx::query("DELETE FROM agent_instance_volume_leases WHERE volume_id = $1")
        .bind(volume_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean lease fixtures");
    sqlx::query("DELETE FROM runs WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean run fixtures");
    sqlx::query("UPDATE agent_instances SET state_volume_id = NULL WHERE id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("detach state volume fixture");
    sqlx::query("DELETE FROM agent_instance_state_volumes WHERE id = $1")
        .bind(volume_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean volume fixture");
}
