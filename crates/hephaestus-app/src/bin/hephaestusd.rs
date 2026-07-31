//! Hephaestus single-node forge and agent-runtime daemon.

use hephaestus_app::{AppConfig, HephaestusApp, OidcConfig, VmBackendConfig};
use jsonwebtoken::{Algorithm, DecodingKey};
use run_runtime_local::LocalRunRuntimeConfig;
use secret_broker::DenyingBrokerAdapter;
use secret_runtime::EphemeralSecretConfig;
use secret_store::LocalKeyProvider;
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    ffi::OsString,
    net::SocketAddr,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use vm_libkrun::LibkrunConfig;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};
use zeroize::Zeroizing;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let config = environment_config()?;
    let app = HephaestusApp::build(config).await?;
    let running = app.start().await?;
    eprintln!("hephaestusd ready at http://{}", running.http_addr());
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}

// One explicit environment-to-config mapping keeps required production
// security settings visible in a single operator-facing location.
#[allow(clippy::too_many_lines)]
fn environment_config() -> Result<AppConfig, Box<dyn Error>> {
    let repository_root = path("HEPHAESTUS_REPOSITORY_ROOT")?;
    let volume_root = path("HEPHAESTUS_VOLUME_ROOT")?;
    let workspace_root = path("HEPHAESTUS_WORKSPACE_ROOT")?;
    let artifact_root = path("HEPHAESTUS_ARTIFACT_ROOT")?;
    let root_image_path = path("HEPHAESTUS_ROOT_IMAGE_PATH")?;
    let root_image_reference = required("HEPHAESTUS_ROOT_IMAGE_REFERENCE")?;
    let runtime_root = path("HEPHAESTUS_RUNTIME_ROOT")?;
    let secret_mount_root = path("HEPHAESTUS_SECRET_RUNTIME_ROOT")?;
    let secret_broker_socket = env::var_os("HEPHAESTUS_SECRET_BROKER_SOCKET")
        .map_or_else(|| runtime_root.join("secret-broker.sock"), PathBuf::from);
    let vm_backend = match env::var("HEPHAESTUS_VM_BACKEND")
        .unwrap_or_else(|_| String::from("libkrun"))
        .as_str()
    {
        "fake" => VmBackendConfig::Fake,
        "fixture" => VmBackendConfig::FixtureResult,
        "libkrun" => {
            let mut config = LibkrunConfig::new(
                &runtime_root,
                vec![root_image_path.clone()],
                vec![volume_root.clone()],
                vec![workspace_root.clone(), secret_mount_root.clone()],
                path("HEPHAESTUS_LIBKRUN_WORKER")?,
                path("HEPHAESTUS_CGROUP_ROOT")?,
            );
            if let Ok(library) = env::var("HEPHAESTUS_LIBKRUN_LIBRARY") {
                config.libkrun_library = OsString::from(library);
            }
            VmBackendConfig::Libkrun(Box::new(config))
        }
        backend => return Err(format!("unsupported HEPHAESTUS_VM_BACKEND {backend:?}").into()),
    };
    let algorithm = match env::var("HEPHAESTUS_OIDC_ALGORITHM")
        .unwrap_or_else(|_| String::from("RS256"))
        .as_str()
    {
        "RS256" => Algorithm::RS256,
        "HS256" => Algorithm::HS256,
        algorithm => return Err(format!("unsupported OIDC algorithm {algorithm:?}").into()),
    };
    let decoding_key = match algorithm {
        Algorithm::RS256 => {
            DecodingKey::from_rsa_pem(&std::fs::read(path("HEPHAESTUS_OIDC_PUBLIC_KEY")?)?)?
        }
        Algorithm::HS256 => {
            DecodingKey::from_secret(required("HEPHAESTUS_OIDC_HS256_SECRET")?.as_bytes())
        }
        _ => return Err(String::from("unsupported configured OIDC algorithm").into()),
    };
    let http_listen = required("HEPHAESTUS_HTTP_LISTEN")?.parse::<SocketAddr>()?;
    let secret_keys = load_secret_keys(
        &path("HEPHAESTUS_SECRET_KEY_DIRECTORY")?,
        required("HEPHAESTUS_SECRET_KEY_REFERENCE")?,
    )?;
    let mut root_images = BTreeMap::new();
    root_images.insert(
        root_image_reference,
        RootFilesystem::Directory {
            host_path: root_image_path,
        },
    );
    let rpc_mediator_secret = required("HEPHAESTUS_RPC_MEDIATOR_SECRET")?;
    Ok(AppConfig {
        database_url: required("HEPHAESTUS_DATABASE_URL")?,
        nats_url: required("HEPHAESTUS_NATS_URL")?,
        http_listen,
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(
            rpc_mediator_secret.as_bytes(),
        ),
        repository_root: repository_root.clone(),
        git_http_backend: path("HEPHAESTUS_GIT_HTTP_BACKEND")?,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: required("HEPHAESTUS_OIDC_ISSUER")?,
            audience: required("HEPHAESTUS_OIDC_AUDIENCE")?,
            algorithm,
            decoding_key,
        },
        volumes: LocalVolumeConfig {
            volume_root,
            transient_runtime_roots: vec![
                runtime_root.clone(),
                workspace_root.clone(),
                secret_mount_root.clone(),
            ],
            host_id: required("HEPHAESTUS_HOST_ID")?,
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: path_or("HEPHAESTUS_MKFS_EXT4", "/usr/bin/mkfs.ext4"),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root: workspace_root.clone(),
            artifact_root: artifact_root.clone(),
            repository_root,
            git_binary: path_or("HEPHAESTUS_GIT_BINARY", "/usr/bin/git"),
            limits: WorkspaceLimits::default(),
        },
        run_runtime: LocalRunRuntimeConfig {
            runtime_root: runtime_root.join("exact-runs"),
            release_artifact_root: artifact_root.join("releases"),
        },
        build_workspace_root: workspace_root.join("isolated-builds"),
        build_timeout: Duration::from_secs(15 * 60),
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: true,
        },
        secret_keys,
        secret_broker_socket,
        secret_broker_adapter: Arc::new(DenyingBrokerAdapter),
        vm_backend,
        root_images,
        runtime_policy: hephaestus_app::RuntimePolicy {
            version: required("HEPHAESTUS_RUNTIME_POLICY_VERSION")?,
            max_vcpus: required("HEPHAESTUS_RUNTIME_MAX_VCPUS")?.parse()?,
            max_memory_mib: required("HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB")?.parse()?,
            allow_broker_only: required("HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY")?.parse()?,
            allow_egress: required("HEPHAESTUS_RUNTIME_ALLOW_EGRESS")?.parse()?,
        },
        agent_state_capacity_bytes: 64 * 1024 * 1024,
        worker_concurrency: 64,
        outbox_poll_interval: Duration::from_millis(50),
        outbox_batch_size: 100,
        startup_timeout: Duration::from_secs(30),
        shutdown_timeout: Duration::from_secs(30),
    })
}

fn required(name: &str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("{name} is required").into())
}

fn path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(required(name)?))
}

fn path_or(name: &str, fallback: &str) -> PathBuf {
    env::var_os(name).map_or_else(|| PathBuf::from(fallback), PathBuf::from)
}

fn load_secret_keys(
    directory: &Path,
    active_reference: String,
) -> Result<LocalKeyProvider, Box<dyn Error>> {
    if !directory.is_absolute() {
        return Err("HEPHAESTUS_SECRET_KEY_DIRECTORY must be absolute".into());
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    let process_uid = std::fs::metadata("/proc/self")?.uid();
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != process_uid
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err("secret key directory must be service-owned mode 0700".into());
    }
    let mut paths = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut keys = Vec::with_capacity(paths.len());
    for key_path in paths {
        let key_metadata = std::fs::symlink_metadata(&key_path)?;
        if key_metadata.file_type().is_symlink()
            || !key_metadata.is_file()
            || key_metadata.uid() != process_uid
            || key_metadata.permissions().mode() & 0o777 != 0o400
        {
            return Err("secret key files must be service-owned regular mode 0400".into());
        }
        let reference = key_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("secret key reference filename must be UTF-8")?
            .to_owned();
        let bytes = Zeroizing::new(std::fs::read(&key_path)?);
        if bytes.len() != 32 {
            return Err("secret key files must contain exactly 32 raw bytes".into());
        }
        keys.push((reference, bytes));
    }
    LocalKeyProvider::new(active_reference, keys).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::load_secret_keys;
    use secret_store::KeyProvider;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn loads_a_strict_multi_version_key_directory() {
        let temporary = tempfile::tempdir().expect("key directory parent");
        let directory = temporary.path().join("keys");
        fs::create_dir(&directory).expect("key directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("key directory mode");
        for (reference, byte) in [("local-v1", 1_u8), ("local-v2", 2_u8)] {
            let path = directory.join(reference);
            fs::write(&path, [byte; 32]).expect("key file");
            fs::set_permissions(path, fs::Permissions::from_mode(0o400)).expect("key file mode");
        }
        let provider =
            load_secret_keys(&directory, String::from("local-v2")).expect("valid key ring");
        assert_eq!(
            provider.active_key_reference().expect("active key"),
            "local-v2"
        );
        assert!(provider.key("local-v1").is_ok());

        fs::set_permissions(
            directory.join("local-v1"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("unsafe key mode");
        assert!(load_secret_keys(&directory, String::from("local-v2")).is_err());
    }
}
