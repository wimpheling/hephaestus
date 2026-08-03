//! Hephaestus single-node forge and agent-runtime daemon.

use builder_catalog_domain::BuilderImageReference;
use hephaestus_app::{AppConfig, HephaestusApp, OidcConfig, VmBackendConfig};
use jsonwebtoken::{Algorithm, DecodingKey};
use run_runtime_local::LocalRunRuntimeConfig;
use secret_broker::DenyingBrokerAdapter;
use secret_runtime::EphemeralSecretConfig;
use secret_store::LocalKeyProvider;
use serde::Deserialize;
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
use vm_trait::{DiskFormat, RootFilesystem};
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
    let backend_name =
        env::var("HEPHAESTUS_VM_BACKEND").unwrap_or_else(|_| String::from("libkrun"));
    let root_images = root_images_from_environment(&backend_name)?;
    let runtime_root = path("HEPHAESTUS_RUNTIME_ROOT")?;
    let secret_mount_root = path("HEPHAESTUS_SECRET_RUNTIME_ROOT")?;
    let secret_broker_socket = env::var_os("HEPHAESTUS_SECRET_BROKER_SOCKET")
        .map_or_else(|| runtime_root.join("secret-broker.sock"), PathBuf::from);
    let vm_backend = match backend_name.as_str() {
        "fake" => VmBackendConfig::Fake,
        "fixture" => VmBackendConfig::FixtureResult,
        "libkrun" => {
            let mut config = LibkrunConfig::new(
                &runtime_root,
                root_images.values().map(root_filesystem_path).collect(),
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

const ROOT_IMAGE_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootImageManifest {
    version: u32,
    roots: BTreeMap<String, RootImageManifestEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RootImageManifestEntry {
    Directory {
        path: PathBuf,
    },
    Disk {
        path: PathBuf,
        format: RootImageDiskFormat,
        read_only: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RootImageDiskFormat {
    Raw,
    Qcow2,
}

fn root_images_from_environment(
    backend_name: &str,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    let manifest = env::var_os("HEPHAESTUS_ROOT_IMAGE_MANIFEST");
    let legacy_path = env::var_os("HEPHAESTUS_ROOT_IMAGE_PATH");
    let legacy_reference = env::var_os("HEPHAESTUS_ROOT_IMAGE_REFERENCE");

    if let Some(manifest) = manifest {
        if legacy_path.is_some() || legacy_reference.is_some() {
            return Err(String::from(
                "HEPHAESTUS_ROOT_IMAGE_MANIFEST cannot be combined with the legacy root image variables",
            )
            .into());
        }
        return load_root_image_manifest(&PathBuf::from(manifest));
    }

    if backend_name == "fixture" {
        let root_image_path = legacy_path.map_or_else(
            || {
                Err(String::from(
                    "HEPHAESTUS_ROOT_IMAGE_PATH is required in fixture mode",
                ))
            },
            Ok,
        )?;
        let root_image_reference = legacy_reference.map_or_else(
            || {
                Err(String::from(
                    "HEPHAESTUS_ROOT_IMAGE_REFERENCE is required in fixture mode",
                ))
            },
            Ok,
        )?;
        return legacy_fixture_root_images(PathBuf::from(root_image_path), root_image_reference);
    }

    Err(String::from("HEPHAESTUS_ROOT_IMAGE_MANIFEST is required outside fixture mode").into())
}

fn legacy_fixture_root_images(
    root_image_path: PathBuf,
    root_image_reference: OsString,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    let mut roots = BTreeMap::new();
    roots.insert(
        os_string_to_string(root_image_reference, "HEPHAESTUS_ROOT_IMAGE_REFERENCE")?,
        RootImageManifestEntry::Directory {
            path: root_image_path,
        },
    );
    validate_root_image_entries(roots)
}

fn load_root_image_manifest(
    manifest_path: &Path,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    if !manifest_path.is_absolute() {
        return Err(String::from("root image manifest path must be absolute").into());
    }
    let manifest: RootImageManifest = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    if manifest.version != ROOT_IMAGE_MANIFEST_VERSION {
        return Err(format!(
            "unsupported root image manifest version {}; expected {}",
            manifest.version, ROOT_IMAGE_MANIFEST_VERSION
        )
        .into());
    }
    validate_root_image_entries(manifest.roots)
}

fn validate_root_image_entries(
    entries: BTreeMap<String, RootImageManifestEntry>,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    if entries.is_empty() {
        return Err(String::from("root image manifest must contain at least one root").into());
    }

    entries
        .into_iter()
        .map(|(reference, entry)| {
            BuilderImageReference::parse(reference.clone()).map_err(|error| {
                format!("root image reference {reference:?} is not digest-pinned: {error}")
            })?;
            let root = match entry {
                RootImageManifestEntry::Directory { path } => {
                    let path = materialized_path(&reference, path, true)?;
                    RootFilesystem::Directory { host_path: path }
                }
                RootImageManifestEntry::Disk {
                    path,
                    format,
                    read_only,
                } => {
                    let path = materialized_path(&reference, path, false)?;
                    let format = match format {
                        RootImageDiskFormat::Raw => DiskFormat::Raw,
                        RootImageDiskFormat::Qcow2 => DiskFormat::Qcow2,
                    };
                    RootFilesystem::Disk {
                        host_path: path,
                        format,
                        read_only,
                    }
                }
            };
            Ok((reference, root))
        })
        .collect()
}

fn materialized_path(
    reference: &str,
    path: PathBuf,
    directory: bool,
) -> Result<PathBuf, Box<dyn Error>> {
    if !path.is_absolute() {
        return Err(
            format!("root image {reference:?} materialization path must be absolute").into(),
        );
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        format!("root image {reference:?} materialization path cannot be inspected: {error}")
    })?;
    if metadata.file_type().is_symlink() {
        return Err(
            format!("root image {reference:?} materialization path must not be a symlink").into(),
        );
    }
    if metadata.is_dir() != directory {
        let expected = if directory { "directory" } else { "disk file" };
        return Err(
            format!("root image {reference:?} materialization path must be a {expected}").into(),
        );
    }
    Ok(std::fs::canonicalize(path)?)
}

fn root_filesystem_path(root: &RootFilesystem) -> PathBuf {
    match root {
        RootFilesystem::Directory { host_path } | RootFilesystem::Disk { host_path, .. } => {
            host_path.clone()
        }
        _ => unreachable!("unsupported root filesystem variant"),
    }
}

fn os_string_to_string(value: OsString, name: &str) -> Result<String, Box<dyn Error>> {
    value
        .into_string()
        .map_err(|_| format!("{name} must contain valid UTF-8").into())
}

#[cfg(test)]
mod manifest_tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn manifest_loads_multiple_materialized_directory_roots() {
        let temporary = tempdir().expect("temporary root");
        let first = temporary.path().join("ubuntu");
        let second = temporary.path().join("rust");
        std::fs::create_dir(&first).expect("first root");
        std::fs::create_dir(&second).expect("second root");
        let manifest = format!(
            r#"{{"version":1,"roots":{{
                "ubuntu@sha256:{}":{{"kind":"directory","path":"{}"}},
                "rust@sha256:{}":{{"kind":"directory","path":"{}"}}
            }}}}"#,
            "a".repeat(64),
            first.display(),
            "b".repeat(64),
            second.display(),
        );
        let manifest_path = temporary.path().join("roots.json");
        std::fs::write(&manifest_path, manifest).expect("manifest");

        let roots = load_root_image_manifest(&manifest_path).expect("valid manifest");
        assert_eq!(roots.len(), 2);
        assert!(roots.contains_key(&format!("ubuntu@sha256:{}", "a".repeat(64))));
        assert!(roots.contains_key(&format!("rust@sha256:{}", "b".repeat(64))));
    }

    #[test]
    fn manifest_rejects_unpinned_references_and_missing_materialization() {
        let temporary = tempdir().expect("temporary root");
        let manifest_path = temporary.path().join("roots.json");
        std::fs::write(
            &manifest_path,
            format!(
                r#"{{"version":1,"roots":{{"ubuntu:latest":{{"kind":"directory","path":"{}"}}}}}}"#,
                temporary.path().display()
            ),
        )
        .expect("manifest");
        assert!(load_root_image_manifest(&manifest_path).is_err());

        std::fs::write(
            &manifest_path,
            format!(
                r#"{{"version":1,"roots":{{"ubuntu@sha256:{}":{{"kind":"directory","path":"{}"}}}}}}"#,
                "a".repeat(64),
                temporary.path().join("missing").display()
            ),
        )
        .expect("manifest");
        assert!(load_root_image_manifest(&manifest_path).is_err());
    }

    #[test]
    fn manifest_supports_explicit_read_only_disk_roots() {
        let temporary = tempdir().expect("temporary root");
        let disk = temporary.path().join("ubuntu.raw");
        File::create(&disk).expect("disk");
        let manifest_path = temporary.path().join("roots.json");
        std::fs::write(
            &manifest_path,
            format!(
                r#"{{"version":1,"roots":{{"ubuntu@sha256:{}":{{"kind":"disk","path":"{}","format":"raw","read_only":true}}}}}}"#,
                "a".repeat(64),
                disk.display()
            ),
        )
        .expect("manifest");

        let roots = load_root_image_manifest(&manifest_path).expect("valid disk manifest");
        assert!(matches!(
            roots.values().next(),
            Some(RootFilesystem::Disk {
                format: DiskFormat::Raw,
                read_only: true,
                ..
            })
        ));
    }

    #[test]
    fn legacy_fixture_pair_still_resolves_to_a_directory_root() {
        let temporary = tempdir().expect("temporary root");
        let roots = legacy_fixture_root_images(
            temporary.path().to_owned(),
            OsString::from(format!("fixture@sha256:{}", "a".repeat(64))),
        )
        .expect("legacy fixture root");
        assert!(matches!(
            roots.values().next(),
            Some(RootFilesystem::Directory { host_path }) if host_path == temporary.path()
        ));
    }
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
