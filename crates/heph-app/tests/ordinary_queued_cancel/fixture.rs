use control_plane_postgres::ControlPlanePool;
use heph_runtime::RootFilesystem;
use hephaestus_app::{
    ManagedInvocationAuthority, ManagedInvocationController, ManagedInvocationInputs,
    ManagedInvocationPools, RuntimePolicy,
};
use release_domain::NetworkAccess;
use release_service::InstanceExecutionConfiguration;
use run_orchestrator::RunOperationGuards;
use run_runtime_local::LocalRunRuntimeConfig;
use secret_key_local::LocalKeyProvider;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration,
};
use tempfile::TempDir;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::VmProvider;
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;

pub struct Fixture {
    pub admin: PgPool,
    pub original: crate::original::Fixture,
    pub provider: Arc<LibkrunProvider>,
    pub expected: LibkrunConfig,
    pub owner: Arc<VolumeRootOwner>,
    _local: Arc<LocalVolumeStore>,
    pub local_config: LocalVolumeConfig,
    pub config: InstanceExecutionConfiguration,
    pub actor_metadata: Arc<PostgresVolumeMetadataRepository>,
    pub metadata: Arc<PostgresVolumeMetadataRepository>,
    pub run_runtime: LocalRunRuntimeConfig,
    pub secrets: PathBuf,
    pub authority: PathBuf,
    pub image: PathBuf,
    // Retained after all owning handles. Never remove these roots on failed work.
    _root: TempDir,
}

impl Fixture {
    pub async fn new(admin: PgPool) -> Self {
        let original = crate::original::Fixture::new(&admin).await;
        control_plane_postgres::verify_qualified_invocation_contract(
            &admin,
            &original.worker,
            &original.application,
        )
        .await
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("vm");
        let volumes = root.path().join("volumes");
        let mounts = root.path().join("mounts");
        let authority = root.path().join("authority");
        let secrets = mounts.join("secrets");
        let run_runtime = LocalRunRuntimeConfig {
            runtime_root: mounts.join("runs"),
            release_artifact_root: root.path().join("releases"),
        };
        for path in [
            &runtime,
            &volumes,
            &mounts,
            &authority,
            &secrets,
            &run_runtime.runtime_root,
            &run_runtime.release_artifact_root,
        ] {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let image = required("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
        let expected = LibkrunConfig::new(
            runtime.clone(),
            vec![image.clone()],
            vec![volumes.clone()],
            vec![mounts.clone()],
            required("HEPHAESTUS_LIBKRUN_WORKER"),
            required("HEPHAESTUS_LIBKRUN_CGROUP_ROOT"),
        );
        let host = "ordinary-queued-cancel-native";
        let provider = Arc::new(LibkrunProvider::new_owned(expected.clone(), host).unwrap());
        let owner = Arc::new(VolumeRootOwner::initialize(&volumes, host).unwrap());
        let local_config = LocalVolumeConfig {
            volume_root: volumes,
            transient_runtime_roots: vec![runtime, mounts],
            host_id: host.into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: required("HEPHAESTUS_PUBLIC139_MKFS_EXT4"),
        };
        let config = InstanceExecutionConfiguration::new(
            provider.owner_scope().unwrap(),
            owner.root().to_str().unwrap().into(),
            owner.namespace_id(),
            release_domain::RuntimePolicy {
                vcpus: 1,
                memory_mib: 128,
                network: NetworkAccess::Disabled,
            },
            "ordinary-queued-cancel/v1".into(),
        )
        .unwrap();
        let actor_metadata = Arc::new(PostgresVolumeMetadataRepository::new(
            original.application.clone(),
        ));
        let metadata = Arc::new(PostgresVolumeMetadataRepository::new(
            original.worker.clone(),
        ));
        let local = Arc::new(
            LocalVolumeStore::new(metadata.clone(), local_config.clone())
                .unwrap()
                .with_owned_metadata(OwnedVolumeMetadata::new(
                    actor_metadata.clone(),
                    metadata.clone(),
                    owner.clone(),
                ))
                .unwrap(),
        );
        Self {
            admin,
            original,
            provider,
            expected,
            owner,
            _local: local,
            local_config,
            config,
            actor_metadata,
            metadata,
            run_runtime,
            secrets,
            authority,
            image,
            _root: root,
        }
    }

    pub async fn controller(&self) -> Arc<ManagedInvocationController> {
        Arc::new(
            ManagedInvocationController::new(ManagedInvocationInputs {
                provider: self.provider.clone(),
                expected_provider: self.expected.clone(),
                volume_owner: self.owner.clone(),
                configuration: self.config.clone(),
                pools: ManagedInvocationPools {
                    admin: self.admin.clone(),
                    worker: self.original.worker.clone(),
                    application: self.original.application.clone(),
                },
                local_volume: self.local_config.clone(),
                run_runtime: self.run_runtime.clone(),
                root_images: BTreeMap::from([(
                    super::instance::image_reference(),
                    RootFilesystem::Directory {
                        host_path: self.image.clone(),
                    },
                )]),
                runtime_policy: RuntimePolicy {
                    version: self.config.platform_version().into(),
                    max_vcpus: 1,
                    max_memory_mib: 128,
                    allow_broker_only: false,
                    allow_egress: false,
                },
                secret_keys: LocalKeyProvider::new(
                    "ordinary-cancel/v1",
                    [("ordinary-cancel/v1", [7_u8; 32])],
                )
                .unwrap(),
                secret_mounts: heph_secret::EphemeralSecretConfig {
                    root: self.secrets.clone(),
                    require_memory_filesystem: false,
                },
                authority: ManagedInvocationAuthority {
                    root: self.authority.clone(),
                    key: [9; 32],
                    session_ttl: Duration::from_secs(30),
                },
                instance_state_capacity_bytes: 16_777_216,
                cleanup_timeout: Duration::from_secs(10),
                operation_guards: Arc::new(RunOperationGuards::default()),
            })
            .await
            .unwrap(),
        )
    }

    pub async fn close(&self) {
        self.owner.validate().unwrap();
        self.provider
            .drain_generation(self.config.provider_scope())
            .await
            .unwrap();
        assert!(
            self.provider
                .attempt_inventory(self.config.provider_scope())
                .await
                .unwrap()
                .attempts
                .is_empty()
        );
        // Only this positively completed scenario closes its original role pools.
        self.original.close().await;
    }
}

pub async fn install(pool: &ControlPlanePool) {
    let migrations = sqlx::migrate!("../../migrations");
    assert_eq!(
        migrations.iter().map(|m| m.version).collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
    );
    migrations.run(pool).await.unwrap();
}

pub async fn role_pool(pool: &PgPool, worker: bool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                sqlx::query(if worker {
                    "SET ROLE hephaestus_worker"
                } else {
                    "SET ROLE hephaestus_app"
                })
                .execute(connection)
                .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap()
}

fn required(name: &str) -> PathBuf {
    std::env::var_os(name).map_or_else(
        || panic!("native test requires actual {name}"),
        PathBuf::from,
    )
}
