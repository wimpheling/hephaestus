//! Fresh persistent allocation and exact configuration for the publication daemon.

#[path = "publication_seed.rs"]
pub(super) mod publication_seed;

use std::{
    collections::BTreeMap,
    fs::{self, File},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use hephaestus_app::{AppConfig, OidcConfig, RegistryConfig, RuntimePolicy, VmBackendConfig};
use jsonwebtoken::{Algorithm, DecodingKey};
use run_runtime_local::LocalRunRuntimeConfig;
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use vm_libkrun::LibkrunConfig;
use vm_trait::{RootFilesystem, VmProvider, VmProviderOwnerScope};
use volume_domain::VolumeRootNamespaceId;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

use super::build_helpers::BuildError;
use publication_seed::GIT_AUDIENCE;

/// Trusted prospective inputs from the fixture operator, never discovered owners.
pub(super) struct PublicationInputs {
    pub(super) parent_database_url: String,
    pub(super) nats_url: String,
    pub(super) fixture_parent: PathBuf,
    pub(super) cgroup_parent: PathBuf,
    pub(super) provider_scope: VmProviderOwnerScope,
    pub(super) volume_namespace: VolumeRootNamespaceId,
    pub(super) rootfs: PathBuf,
    pub(super) image_input: PathBuf,
    pub(super) image_input_sha256: [u8; 32],
    pub(super) worker: PathBuf,
    pub(super) git_binary: PathBuf,
    pub(super) git_http_backend: PathBuf,
    pub(super) git_pre_receive_hook: PathBuf,
    pub(super) mkfs_ext4: PathBuf,
    pub(super) service_uid: u32,
    pub(super) service_gid: u32,
    pub(super) policy: RuntimePolicy,
    pub(super) registry: RegistryConfig,
    pub(super) oidc_issuer: String,
    pub(super) oidc_signing_secret: Vec<u8>,
    pub(super) rpc_mediator_signing_key: [u8; 32],
    pub(super) handoff_key: [u8; 32],
    pub(super) secret_keys: secret_key_local::LocalKeyProvider,
}

impl PublicationInputs {
    fn validate(&self) -> Result<(), BuildError> {
        let namespace = uuid::Uuid::parse_str(self.provider_scope.namespace())?;
        assert!(!namespace.is_nil());
        assert_eq!(namespace.to_string(), self.provider_scope.namespace());
        assert_ne!(self.service_uid, 0);
        assert_eq!(self.service_uid, rustix::process::geteuid().as_raw());
        assert_eq!(self.service_gid, rustix::process::getegid().as_raw());
        assert!(self.policy.max_vcpus >= 1 && self.policy.max_memory_mib >= 256);
        assert!(!self.policy.version.trim().is_empty());
        assert!(self.oidc_signing_secret.len() >= 32);
        assert_ne!(self.rpc_mediator_signing_key, [0; 32]);
        assert_ne!(self.handoff_key, [0; 32]);
        assert_ne!(self.image_input_sha256, [0; 32]);
        for path in [
            &self.fixture_parent,
            &self.cgroup_parent,
            &self.rootfs,
            &self.image_input,
            &self.worker,
            &self.git_binary,
            &self.git_http_backend,
            &self.git_pre_receive_hook,
            &self.mkfs_ext4,
        ] {
            assert!(path.is_absolute());
            assert_eq!(fs::canonicalize(path)?, *path);
        }
        assert_eq!(
            self.git_pre_receive_hook.file_name().unwrap(),
            "pre-receive"
        );
        assert!(self.cgroup_parent.join("cgroup.controllers").is_file());
        Ok(())
    }
}

/// Keeps allocation history and the independent pool through errors/cancellation.
///
/// No Drop implementation removes directories or databases. Future C cleanup
/// requires separately verified shutdown and physical drain, including failures.
pub(super) struct FreshSetup {
    pub(super) root: PathBuf,
    pub(super) cgroup_root: PathBuf,
    pub(super) image: Value,
    pub(super) database_name: String,
    pub(super) maintenance_url: String,
    pub(super) database_url: String,
    pin: File,
    rootfs_pin: File,
    rootfs_path: PathBuf,
    pool: Option<PgPool>,
}

impl FreshSetup {
    /// Allocates fresh history directly; supplied roots are never adopted.
    pub(super) fn allocate(inputs: &PublicationInputs) -> Result<Self, BuildError> {
        inputs.validate()?;
        let image = publication_seed::image_input(inputs)?;
        let directory = tempfile::Builder::new()
            .prefix("heph-ql-")
            .tempdir_in(&inputs.fixture_parent)?;
        let root = directory.keep();
        let pin = File::open(&root)?;
        let cgroup_root = inputs
            .cgroup_parent
            .join(format!("ql-{}", inputs.provider_scope.namespace()));
        let database_name = format!(
            "hephaestus_qualified_local_{}",
            uuid::Uuid::new_v4().simple()
        );
        let mut maintenance = url::Url::parse(&inputs.parent_database_url)?;
        maintenance.set_path("/postgres");
        let mut database = url::Url::parse(&inputs.parent_database_url)?;
        database.set_path(&format!("/{database_name}"));
        fs::write(
            root.join("allocation.json"),
            serde_json::to_vec_pretty(&json!({
                "kind":"prospective_fixture_allocation_not_a_drain_receipt",
                "database_name":database_name,"root":root,"cgroup_root":cgroup_root,
                "provider_namespace":inputs.provider_scope.namespace(),
                "host":inputs.provider_scope.host_id(),"volume_namespace":inputs.volume_namespace,
                "service_uid":inputs.service_uid,"service_gid":inputs.service_gid,
            }))?,
        )?;
        let setup = Self {
            root,
            cgroup_root,
            image,
            database_name,
            maintenance_url: maintenance.to_string(),
            database_url: database.to_string(),
            pin,
            rootfs_pin: File::open(&inputs.rootfs)?,
            rootfs_path: inputs.rootfs.clone(),
            pool: None,
        };
        setup.validate_root()?;
        Ok(setup)
    }

    pub(super) fn validate_root(&self) -> Result<(), BuildError> {
        assert_eq!(fs::canonicalize(&self.root)?, self.root);
        let actual = fs::symlink_metadata(&self.root)?;
        let retained = self.pin.metadata()?;
        assert!(actual.is_dir() && !actual.file_type().is_symlink());
        assert_eq!(
            (actual.dev(), actual.ino(), actual.uid()),
            (retained.dev(), retained.ino(), retained.uid())
        );
        assert_eq!(actual.mode() & 0o022, 0);
        publication_seed::validate_rootfs(&self.rootfs_path, &self.rootfs_pin)
    }

    pub(super) fn create_paths(&self) -> Result<(), BuildError> {
        self.validate_root()?;
        for name in [
            "vm-runtime",
            "volumes",
            "workspaces",
            "artifacts",
            "run-runtime",
            "release-artifacts",
            "isolated-builds",
            "secret-mounts",
            "handoffs",
            "repositories",
        ] {
            let path = self.root.join(name);
            fs::create_dir(&path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        fs::create_dir(&self.cgroup_root)?;
        fs::set_permissions(&self.cgroup_root, fs::Permissions::from_mode(0o700))?;
        self.validate_root()
    }

    pub(super) async fn create_database(&mut self) -> Result<(), BuildError> {
        <sqlx::Postgres as sqlx::migrate::MigrateDatabase>::create_database(&self.database_url)
            .await?;
        self.pool = Some(
            PgPoolOptions::new()
                .max_connections(5)
                .connect(&self.database_url)
                .await?,
        );
        let migrations = sqlx::migrate!("../../migrations");
        assert_eq!(migrations.iter().last().unwrap().version, 107);
        migrations.run(self.pool()?).await?;
        let installed: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE success ORDER BY version",
        )
        .fetch_all(self.pool()?)
        .await?;
        let expected = migrations
            .iter()
            .map(|migration| (migration.version, migration.checksum.to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(installed, expected, "complete immutable public107 history");
        Ok(())
    }

    pub(super) fn pool(&self) -> Result<&PgPool, BuildError> {
        self.pool
            .as_ref()
            .ok_or_else(|| std::io::Error::other("publication admin pool is unavailable").into())
    }

    pub(super) fn image_reference(&self) -> &str {
        self.image["catalog_entry"]["image_reference"]
            .as_str()
            .unwrap()
    }

    pub(super) fn provider_config(&self, inputs: &PublicationInputs) -> LibkrunConfig {
        let mut config = LibkrunConfig::new(
            self.root.join("vm-runtime"),
            vec![inputs.rootfs.clone()],
            vec![self.root.join("volumes")],
            [
                "isolated-builds",
                "workspaces",
                "run-runtime",
                "secret-mounts",
            ]
            .map(|name| self.root.join(name))
            .into(),
            inputs.worker.clone(),
            self.cgroup_root.clone(),
        );
        config.service_uid = inputs.service_uid;
        config.service_gid = inputs.service_gid;
        config.broker_socket_path = Some(self.root.join("secret-broker.sock"));
        config.runtime_git_socket_path = Some(self.root.join("runtime-git.sock"));
        config
    }

    pub(super) fn local_config(&self, inputs: &PublicationInputs) -> LocalVolumeConfig {
        LocalVolumeConfig {
            volume_root: self.root.join("volumes"),
            transient_runtime_roots: vec![
                self.root.join("vm-runtime"),
                self.root.join("workspaces"),
            ],
            host_id: inputs.provider_scope.host_id().to_owned(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: inputs.mkfs_ext4.clone(),
        }
    }

    pub(super) fn runtime_config(&self) -> LocalRunRuntimeConfig {
        LocalRunRuntimeConfig {
            runtime_root: self.root.join("run-runtime"),
            release_artifact_root: self.root.join("release-artifacts"),
        }
    }

    pub(super) fn app_config(
        &self,
        inputs: &PublicationInputs,
        provider: Arc<dyn VmProvider>,
    ) -> AppConfig {
        AppConfig {
            database_url: self.database_url.clone(),
            nats_url: inputs.nats_url.clone(),
            http_listen: "127.0.0.1:0".parse().unwrap(),
            rpc_mediator_signing_key: inputs.rpc_mediator_signing_key,
            repository_root: self.root.join("repositories"),
            git_http_backend: inputs.git_http_backend.clone(),
            git_pre_receive_hook: inputs.git_pre_receive_hook.clone(),
            git_http_limits: git_http::GitHttpLimits::default(),
            oidc: OidcConfig {
                issuer: inputs.oidc_issuer.clone(),
                audience: GIT_AUDIENCE.to_owned(),
                algorithm: Algorithm::HS256,
                decoding_key: DecodingKey::from_secret(&inputs.oidc_signing_secret),
            },
            registry: inputs.registry.clone(),
            gateway_edge: None,
            volumes: self.local_config(inputs),
            workspaces: LocalWorkspaceConfig {
                workspace_root: self.root.join("workspaces"),
                artifact_root: self.root.join("artifacts"),
                repository_root: self.root.join("repositories"),
                git_binary: inputs.git_binary.clone(),
                limits: WorkspaceLimits::default(),
            },
            run_runtime: self.runtime_config(),
            runtime_authority_handoff_root: self.root.join("handoffs"),
            runtime_authority_handoff_key: inputs.handoff_key,
            runtime_authority_session_ttl: Duration::from_secs(3600),
            build_workspace_root: self.root.join("isolated-builds"),
            build_timeout: Duration::from_secs(90),
            secret_mounts: secret_runtime::EphemeralSecretConfig {
                root: self.root.join("secret-mounts"),
                require_memory_filesystem: false,
            },
            secret_keys: inputs.secret_keys.clone(),
            secret_broker_socket: self.root.join("secret-broker.sock"),
            secret_broker_adapter: Arc::new(secret_broker::DenyingBrokerAdapter),
            managed_backend: None,
            vm_backend: VmBackendConfig::CustomWithRuntimeGitSocket {
                provider,
                runtime_git_socket_path: self.root.join("runtime-git.sock"),
            },
            root_images: BTreeMap::from([(
                self.image_reference().to_owned(),
                RootFilesystem::Directory {
                    host_path: inputs.rootfs.clone(),
                },
            )]),
            oci_builder: None,
            runtime_policy: inputs.policy.clone(),
            agent_state_capacity_bytes: 16 * 1024 * 1024,
            worker_concurrency: 2,
            outbox_poll_interval: Duration::from_millis(10),
            outbox_batch_size: 20,
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(10),
        }
    }
}
