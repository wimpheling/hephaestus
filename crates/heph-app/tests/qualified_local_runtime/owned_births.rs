//! Genuine owned births over the retained actual backend root and identity.

use std::{os::unix::fs::MetadataExt, path::PathBuf, sync::Arc};

use release_domain::ContentHash;
use sqlx::PgPool;
use uuid::Uuid;
use volume_local::{LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPhase, OwnedProvisioningExpectation,
    OwnedVolumeRegistration, VolumeOwnedRegistrationRepository, VolumeRegistration,
};

use super::bootstrap::Bootstrap;

pub struct OwnedBirths {
    pub root: PathBuf,
    pub owner: Arc<VolumeRootOwner>,
    pub store: Arc<LocalVolumeStore>,
    pub metadata: Arc<PostgresVolumeMetadataRepository>,
    pub files: Vec<(Uuid, PathBuf, u64, u64, ContentHash)>,
}
impl OwnedBirths {
    pub async fn new(bootstrap: &Bootstrap) -> Self {
        bootstrap.published.validate_identity();
        let root = bootstrap.published.local.volume_root.clone();
        let owner = bootstrap.published.owner.clone();
        let actor = Arc::new(PostgresVolumeMetadataRepository::new(bootstrap.app.clone()));
        let worker = Arc::new(
            PostgresVolumeMetadataRepository::new(bootstrap.worker.clone())
                .with_qualified_invocation_configuration(bootstrap.published.configuration.clone()),
        );
        let store = Arc::new(
            LocalVolumeStore::new(worker.clone(), bootstrap.published.local.clone())
                .unwrap()
                .with_owned_metadata(OwnedVolumeMetadata::new(
                    actor.clone(),
                    worker.clone(),
                    owner.clone(),
                ))
                .unwrap()
                .with_run_metadata(worker.clone()),
        );
        let mut result = Self {
            root,
            owner,
            store: store.clone(),
            metadata: worker,
            files: Vec::new(),
        };
        for index in 0..bootstrap.input.volumes().len() {
            result
                .birth(&bootstrap.published.pool, bootstrap, &actor, &store, index)
                .await;
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
            bootstrap.published.local.host_id.clone(),
            self.root.clone(),
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

pub async fn complete_owned(
    bootstrap: &Bootstrap,
    claim: recipe_application::EffectClaim,
    mut custody: volume_local::OwnedReadyCustody,
) {
    custody.revalidate().unwrap();
    let repository = bootstrap.recipe.clone();
    let actor = bootstrap.actor.clone();
    let worker = bootstrap.worker.clone();
    // Protected completion owns the non-clone native guard through the actual
    // worker record and actor commit, even if the calling fixture is cancelled.
    tokio::spawn(async move {
        custody.revalidate().unwrap();
        recipe_postgres::PostgresEffectVerificationRecorder::new(worker)
            .record(
                &claim,
                claim.provenance.attempt_id,
                recipe_postgres::VerifiedOutcome::Applied,
            )
            .await
            .unwrap();
        custody.revalidate().unwrap();
        repository
            .complete_resource_effect(
                &actor,
                &claim,
                recipe_application::EffectEvidence {
                    identity: claim.identity,
                    input_hash: claim.input_hash,
                    outcome: recipe_application::EffectOutcome::Applied,
                },
            )
            .await
            .unwrap();
        custody.revalidate().unwrap();
    })
    .await
    .unwrap();
}
