use super::worker::Worker;
use crate::seals;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration};
use tempfile::TempDir;
use uuid::Uuid;
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPhase, OwnedBackingPurpose, OwnedProvisioningClaim,
    VolumeError, VolumeOwnedProvisioningRepository, VolumeOwnedRegistrationRepository,
};

pub struct Fixture {
    pub sql: seals::Fixture,
    pub root: TempDir,
    pub tools: TempDir,
    pub owner: Arc<VolumeRootOwner>,
    pub worker: Arc<PostgresVolumeMetadataRepository>,
    pub identity: AuthenticatedIdentity,
    pub request: BeginOwnedProvisioning,
    pub store: LocalVolumeStore,
    pub real_mkfs: PathBuf,
}
impl Fixture {
    pub async fn new(pool: &PgPool, fail_ready: bool) -> Self {
        seals::checkpoint(111)
            .run(pool)
            .await
            .expect("actual versioned ordered private108..111");
        let sql = seals::seed(pool).await;
        let actor = Arc::new(sql.actor.clone());
        let worker = Arc::new(sql.worker.clone());
        let identity = seals::identity(sql.owner);
        let receipt = actor
            .register_owned(&identity, &seals::intent(&sql))
            .await
            .expect("actual110 owned registration seal and original correlation");
        let root = TempDir::new().expect("private physical root");
        let tools = TempDir::new().expect("trusted fixture tools");
        let owner = Arc::new(
            VolumeRootOwner::initialize(root.path(), "owned-local-pg-host")
                .expect("actual pinned physical namespace"),
        );
        let real_mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("install actual mkfs.ext4");
        let real_fsck = real_mkfs.parent().expect("tools parent").join("e2fsck");
        assert!(real_fsck.is_file(), "actual e2fsck required");
        wrapper(&tools, "mkfs.ext4", &real_mkfs, "formats");
        wrapper(&tools, "e2fsck", &real_fsck, "fsck-args");
        let purpose = OwnedBackingPurpose::new(
            receipt,
            "owned-local-pg-host".into(),
            root.path().into(),
            owner.namespace_id(),
            1,
        )
        .expect("exact SQL/host purpose");
        let request = BeginOwnedProvisioning {
            purpose,
            operation_id: Uuid::new_v4(),
            expected_generation: 0,
        };
        let store = LocalVolumeStore::new(
            worker.clone(),
            LocalVolumeConfig {
                volume_root: root.path().into(),
                transient_runtime_roots: Vec::new(),
                host_id: "owned-local-pg-host".into(),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: tools.path().join("mkfs.ext4"),
            },
        )
        .expect("same canonical Local backend")
        .with_owned_metadata(OwnedVolumeMetadata::new(
            actor,
            Arc::new(Worker::new(worker.clone(), fail_ready)),
            owner.clone(),
        ))
        .expect("distinct actor and worker SQL adapters");
        Self {
            sql,
            root,
            tools,
            owner,
            worker,
            identity,
            request,
            store,
            real_mkfs,
        }
    }
    pub fn format_count(&self) -> usize {
        fs::read_to_string(self.tools.path().join("formats"))
            .unwrap_or_default()
            .lines()
            .count()
    }
    pub async fn format_commit_gap(&self) -> OwnedProvisioningClaim {
        let error = self
            .store
            .provision_owned(&self.identity, &self.request)
            .await
            .expect_err("lose only final worker Ready record after real physical format");
        assert!(
            matches!(
                error,
                VolumeError::ProvisioningUncertain(
                    "fixture failed final Ready SQL record before delegation"
                )
            ),
            "exact injected final record gap: {error:?}"
        );
        let admitted = self
            .sql
            .actor
            .begin_owned_provisioning(&self.identity, &self.request)
            .await
            .expect("live original admission replay for exact persisted claim");
        let current = self
            .worker
            .owned_reconciliation_context(&admitted.claim)
            .await
            .expect("actual persisted format intent");
        assert_eq!(
            current
                .observation
                .expect("physical predecessor recorded in SQL")
                .fields()
                .phase,
            OwnedBackingPhase::FormatIntent
        );
        assert_eq!(self.format_count(), 1);
        assert_not_ready(self).await;
        admitted.claim
    }
}
fn wrapper(tools: &TempDir, name: &str, actual: &std::path::Path, log: &str) {
    let path = tools.path().join(name);
    // All paths are task-created tempfile/tool paths, never application inputs.
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        tools.path().join(log).display(),
        actual.display()
    );
    fs::write(&path, script).expect("native executable wrapper");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("private executable");
}
pub async fn assert_not_ready(fixture: &Fixture) {
    let state:(String,String,i64)=sqlx::query_as("SELECT state,provisioning_state,(SELECT count(*) FROM volume_owned_backing_observations WHERE volume_id=volume.id AND phase='ready') FROM agent_instance_state_volumes volume WHERE id=$1")
        .bind(fixture.request.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&fixture.sql.pool).await.expect("actual canonical resource state");
    assert_ne!(state.0, "ready");
    assert_ne!(state.1, "ready");
    assert_eq!(state.2, 0);
}
