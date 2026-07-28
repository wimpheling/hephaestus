//! Hephaestus single-node forge and agent-runtime daemon.

use hephaestus_app::{AppConfig, HephaestusApp, OidcConfig, VmBackendConfig};
use jsonwebtoken::{Algorithm, DecodingKey};
use std::{
    collections::BTreeMap, env, error::Error, ffi::OsString, net::SocketAddr, path::PathBuf,
    time::Duration,
};
use vm_libkrun::LibkrunConfig;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

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

fn environment_config() -> Result<AppConfig, Box<dyn Error>> {
    let repository_root = path("HEPHAESTUS_REPOSITORY_ROOT")?;
    let volume_root = path("HEPHAESTUS_VOLUME_ROOT")?;
    let workspace_root = path("HEPHAESTUS_WORKSPACE_ROOT")?;
    let artifact_root = path("HEPHAESTUS_ARTIFACT_ROOT")?;
    let root_image_path = path("HEPHAESTUS_ROOT_IMAGE_PATH")?;
    let root_image_reference = required("HEPHAESTUS_ROOT_IMAGE_REFERENCE")?;
    let runtime_root = path("HEPHAESTUS_RUNTIME_ROOT")?;
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
                vec![workspace_root.clone()],
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
    let mut root_images = BTreeMap::new();
    root_images.insert(
        root_image_reference,
        RootFilesystem::Directory {
            host_path: root_image_path,
        },
    );
    Ok(AppConfig {
        database_url: required("HEPHAESTUS_DATABASE_URL")?,
        nats_url: required("HEPHAESTUS_NATS_URL")?,
        http_listen,
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
            transient_runtime_roots: vec![runtime_root, workspace_root.clone()],
            host_id: required("HEPHAESTUS_HOST_ID")?,
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: path_or("HEPHAESTUS_MKFS_EXT4", "/usr/bin/mkfs.ext4"),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root,
            artifact_root,
            repository_root,
            git_binary: path_or("HEPHAESTUS_GIT_BINARY", "/usr/bin/git"),
            limits: WorkspaceLimits::default(),
        },
        vm_backend,
        root_images,
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
