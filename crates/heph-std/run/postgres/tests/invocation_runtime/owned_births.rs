//! Genuine Local births share one physical root; no synthetic Ready records.

use std::{os::unix::fs::MetadataExt, path::PathBuf, sync::Arc, time::Duration};

use sqlx::PgPool;
use tempfile::TempDir;
use uuid::Uuid;
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPhase, OwnedProvisioningExpectation,
    OwnedVolumeRegistration, VolumeOwnedRegistrationRepository, VolumeRegistration,
};

use super::{bootstrap::Bootstrap, completion::complete_owned};
use release_domain::ContentHash;

pub struct OwnedBirths {
    pub root: TempDir,
    pub owner: Arc<VolumeRootOwner>,
    pub files: Vec<(Uuid, PathBuf, u64, u64, ContentHash)>,
}
impl OwnedBirths {
    pub async fn new(pool: &PgPool, bootstrap: &Bootstrap, count: usize) -> Self {
        let root = TempDir::new().unwrap();
        let owner = Arc::new(VolumeRootOwner::initialize(root.path(), "fixture-host").unwrap());
        let actor = Arc::new(PostgresVolumeMetadataRepository::new(bootstrap.app.clone()));
        let worker = Arc::new(PostgresVolumeMetadataRepository::new(
            bootstrap.worker.clone(),
        ));
        let mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("native mkfs.ext4/e2fsck required");
        let store = LocalVolumeStore::new(
            worker.clone(),
            LocalVolumeConfig {
                volume_root: root.path().into(),
                transient_runtime_roots: Vec::new(),
                host_id: "fixture-host".to_owned(),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: mkfs,
            },
        )
        .unwrap()
        .with_owned_metadata(OwnedVolumeMetadata::new(
            actor.clone(),
            worker,
            owner.clone(),
        ))
        .unwrap();
        let mut result = Self {
            root,
            owner,
            files: Vec::new(),
        };
        for index in 0..count {
            result.birth(pool, bootstrap, &actor, &store, index).await;
        }
        result
    }
    async fn birth(
        &mut self,
        pool: &PgPool,
        bootstrap: &Bootstrap,
        actor: &PostgresVolumeMetadataRepository,
        store: &LocalVolumeStore,
        index: usize,
    ) {
        let claim = bootstrap.claim(&format!("data{index:02}")).await;
        let prepared = bootstrap
            .recipe
            .prepare_owned_volume_creation(&bootstrap.actor, &claim)
            .await
            .unwrap();
        let registration = OwnedVolumeRegistration::new(
            VolumeRegistration::new(
                prepared.volume_id,
                prepared.project_id,
                prepared.capacity_bytes,
                prepared.filesystem_uuid,
            )
            .unwrap(),
            prepared.creation,
        );
        let receipt = actor
            .register_owned(&bootstrap.actor, &registration)
            .await
            .unwrap();
        let expected = OwnedProvisioningExpectation::new(
            registration,
            "fixture-host".to_owned(),
            self.root.path().into(),
            self.owner.namespace_id(),
        )
        .unwrap();
        let purpose = expected.purpose(receipt).unwrap();
        let path = purpose.canonical_path().clone();
        let request = BeginOwnedProvisioning {
            purpose,
            operation_id: expected.first_operation_id(),
            expected_generation: 0,
        };
        let ready = store
            .provision_owned(&bootstrap.actor, &request)
            .await
            .expect("actual mkfs, pinned journal and readonly initial fsck");
        let custody = store
            .verify_owned_ready(&ready.claim)
            .await
            .expect("actual pinned first-Ready custody and readonly fsck before106 completion");
        let fields = custody.context().observation.as_ref().unwrap().fields();
        assert_eq!(fields.phase, OwnedBackingPhase::Ready);
        assert!(fields.filesystem.as_ref().unwrap().clean);
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(
            (metadata.dev(), metadata.ino()),
            (fields.backing_device, fields.backing_inode)
        );
        assert_eq!(metadata.len(), prepared.capacity_bytes);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[1080..1082], &[0x53, 0xef]);
        assert_eq!(&bytes[1128..1144], prepared.filesystem_uuid.as_bytes());
        let exact:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_owned_volume_preparations preparation JOIN volume_creation_seals seal ON seal.volume_id=preparation.volume_id JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=seal.volume_id JOIN volume_owned_backing_observations observation ON observation.volume_id=seal.volume_id WHERE preparation.volume_id=$1 AND observation.id=$2 AND observation.phase='ready' AND purpose.canonical_bytes=$3)")
            .bind(prepared.volume_id.as_uuid()).bind(fields.id).bind(request.purpose.canonical_bytes()).fetch_one(pool).await.unwrap();
        assert!(
            exact,
            "physical receipt matches original durable owning preparation and seal"
        );
        let hash = ContentHash::digest(&bytes);
        complete_owned(bootstrap, claim, custody).await;
        self.files.push((
            prepared.volume_id.as_uuid(),
            path,
            metadata.dev(),
            metadata.ino(),
            hash,
        ));
    }
    pub fn assert_unchanged(&self) {
        for (_, path, device, inode, hash) in &self.files {
            let metadata = std::fs::metadata(path).unwrap();
            assert_eq!((metadata.dev(), metadata.ino()), (*device, *inode));
            assert_eq!(ContentHash::digest(&std::fs::read(path).unwrap()), *hash);
        }
    }
}
