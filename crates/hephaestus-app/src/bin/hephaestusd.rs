//! Hephaestus single-node forge and agent-runtime daemon.

use builder_catalog_domain::OciImageReference;
use hephaestus_app::{
    AppConfig, HephaestusApp, OciBuilderWorkerConfig, OidcConfig, RegistryConfig, VmBackendConfig,
};
use jsonwebtoken::{Algorithm, DecodingKey};
use oci_builder_runtime_local::LocalOciRuntimeConfig;
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
    let oci_builder = oci_builder_from_environment(&repository_root, &runtime_root)?;
    let secret_mount_root = path("HEPHAESTUS_SECRET_RUNTIME_ROOT")?;
    let secret_broker_socket = env::var_os("HEPHAESTUS_SECRET_BROKER_SOCKET")
        .map_or_else(|| runtime_root.join("secret-broker.sock"), PathBuf::from);
    let runtime_authority_handoff_root = env::var_os("HEPHAESTUS_RUNTIME_AUTHORITY_HANDOFF_ROOT")
        .map_or_else(|| runtime_root.join("authority-handoffs"), PathBuf::from);
    let runtime_authority_handoff_key =
        fixed_key(path("HEPHAESTUS_RUNTIME_AUTHORITY_HANDOFF_KEY_FILE")?)?;
    let vm_backend = match backend_name.as_str() {
        "fake" => VmBackendConfig::Fake,
        "fixture" => VmBackendConfig::FixtureResult,
        "libkrun" => {
            let mut image_roots: Vec<_> = root_images.values().map(root_filesystem_path).collect();
            if let Some(worker) = &oci_builder {
                image_roots.push(worker.rootfs_root.clone());
            }
            let mut config = LibkrunConfig::new(
                &runtime_root,
                image_roots,
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
    let registry_private_key = Zeroizing::new(std::fs::read(path(
        "HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY",
    )?)?);
    let registry_authority_text = required("HEPHAESTUS_REGISTRY_SERVICE")?;
    let registry_authority =
        registry_domain::RegistryAuthority::parse(registry_authority_text.clone())?;
    let registry_token_issuer = registry_token::RegistryTokenIssuer::new(
        required("HEPHAESTUS_REGISTRY_TOKEN_ISSUER")?.parse()?,
        registry_authority_text.parse()?,
        registry_token::SigningKey::rs256_pem(
            required("HEPHAESTUS_REGISTRY_TOKEN_KEY_ID")?.parse()?,
            &registry_private_key,
        )?,
        registry_token::TokenLifetime::new(optional_u64(
            "HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS",
            300,
        )?)?,
    );
    let registry_notification_callback = Zeroizing::new(
        std::fs::read_to_string(path(
            "HEPHAESTUS_REGISTRY_NOTIFICATION_CALLBACK_TOKEN_FILE",
        )?)?
        .trim()
        .to_owned(),
    );
    Ok(AppConfig {
        database_url: required("HEPHAESTUS_DATABASE_URL")?,
        nats_url: required("HEPHAESTUS_NATS_URL")?,
        http_listen,
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(
            rpc_mediator_secret.as_bytes(),
        ),
        repository_root: repository_root.clone(),
        git_http_backend: path("HEPHAESTUS_GIT_HTTP_BACKEND")?,
        git_pre_receive_hook: path("HEPHAESTUS_GIT_PRE_RECEIVE_HOOK")?,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: required("HEPHAESTUS_OIDC_ISSUER")?,
            audience: required("HEPHAESTUS_OIDC_AUDIENCE")?,
            algorithm,
            decoding_key,
        },
        registry: RegistryConfig {
            token_issuer: Arc::new(registry_token_issuer),
            notification_callback: registry_notification::CallbackCredential::parse(
                &*registry_notification_callback,
            )?,
            zot: registry_zot::ZotClientConfig::new(
                registry_authority,
                &required("HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN")?,
            )?,
            reconciliation_lease: Duration::from_secs(optional_u64(
                "HEPHAESTUS_REGISTRY_RECONCILIATION_LEASE_SECONDS",
                30,
            )?),
            reconciliation_interval: Duration::from_millis(optional_u64(
                "HEPHAESTUS_REGISTRY_RECONCILIATION_INTERVAL_MILLISECONDS",
                5_000,
            )?),
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
        runtime_authority_handoff_root,
        runtime_authority_handoff_key,
        runtime_authority_session_ttl: Duration::from_secs(optional_u64(
            "HEPHAESTUS_RUNTIME_AUTHORITY_SESSION_TTL_SECONDS",
            3_600,
        )?),
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
        oci_builder,
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

fn fixed_key(path: PathBuf) -> Result<[u8; 32], Box<dyn Error>> {
    let bytes = Zeroizing::new(std::fs::read(path)?);
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| "runtime authority handoff key must contain exactly 32 bytes".into())
}

fn required(name: &str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("{name} is required").into())
}

fn path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(required(name)?))
}

fn oci_builder_from_environment(
    repository_root: &Path,
    runtime_root: &Path,
) -> Result<Option<OciBuilderWorkerConfig>, Box<dyn Error>> {
    let Some(rootfs_root) = env::var_os("HEPHAESTUS_OCI_BUILDER_ROOTFS_ROOT") else {
        return Ok(None);
    };
    let base_layout_manifest = path("HEPHAESTUS_OCI_BUILDER_BASE_LAYOUT_MANIFEST")?;
    if !base_layout_manifest.is_absolute() {
        return Err(String::from("OCI base-layout manifest path must be absolute").into());
    }
    let image_layouts: BTreeMap<String, PathBuf> =
        serde_json::from_slice(&std::fs::read(base_layout_manifest)?)?;
    if image_layouts.is_empty() {
        return Err(String::from("OCI base-layout manifest must not be empty").into());
    }
    let host_id = required("HEPHAESTUS_HOST_ID")?;
    let preparation_worker_name = env::var("HEPHAESTUS_OCI_BUILDER_PREPARATION_WORKER")
        .unwrap_or_else(|_| format!("oci-preparation-{host_id}"));
    let materialization_worker_name = env::var("HEPHAESTUS_OCI_BUILDER_MATERIALIZATION_WORKER")
        .unwrap_or_else(|_| format!("oci-rootfs-{host_id}"));
    let output_root = path("HEPHAESTUS_OCI_BUILDER_OUTPUT_ROOT")?;
    let registry_authority =
        registry_domain::RegistryAuthority::parse(required("HEPHAESTUS_REGISTRY_SERVICE")?)?;
    let runtime = LocalOciRuntimeConfig {
        repository_root: repository_root.to_path_buf(),
        checkout_root: path("HEPHAESTUS_OCI_BUILDER_CHECKOUT_ROOT")?,
        image_layouts,
        output_root: output_root.clone(),
        git_binary: path_or("HEPHAESTUS_GIT_BINARY", "/usr/bin/git"),
        tar_binary: path_or("HEPHAESTUS_TAR_BINARY", "/usr/bin/tar"),
        buildah_binary: path_or("HEPHAESTUS_BUILDAH_BINARY", "/usr/bin/buildah"),
        trivy_binary: path_or("HEPHAESTUS_TRIVY_BINARY", "/usr/bin/trivy"),
        umoci_binary: path_or("HEPHAESTUS_UMOCI_BINARY", "/usr/bin/umoci"),
        buildah_output_prefix: env::var("HEPHAESTUS_OCI_BUILDER_OUTPUT_PREFIX")
            .unwrap_or_else(|_| String::from("heph-builder")),
    };
    let publisher = registry_publisher::PublisherConfiguration::new(
        registry_authority,
        &output_root,
        &path("HEPHAESTUS_REGISTRY_CREDENTIAL_ROOT")?,
        &path_or("HEPHAESTUS_SKOPEO", "/usr/bin/skopeo"),
        &path_or("HEPHAESTUS_ORAS", "/usr/bin/oras"),
    )?;
    Ok(Some(OciBuilderWorkerConfig {
        runtime,
        publication_tooling: oci_builder_runtime_local::ForgeZotPublicationConfig {
            syft_binary: path_or("HEPHAESTUS_SYFT_BINARY", "/usr/bin/syft"),
            syft_config: path("HEPHAESTUS_SYFT_CONFIG")?,
        },
        publisher,
        publication_policy_version: registry_domain::PolicyVersion::parse(
            env::var("HEPHAESTUS_REGISTRY_POLICY_VERSION")
                .unwrap_or_else(|_| String::from("registry/v1")),
        )?,
        publication_policy: registry_domain::SupplyChainPolicy::without_signature(),
        preparation_worker_name,
        materialization_worker_name,
        rootfs_root: PathBuf::from(rootfs_root),
        root_manifest: env::var_os("HEPHAESTUS_OCI_BUILDER_ROOT_MANIFEST").map_or_else(
            || runtime_root.join("repository-builder-roots.json"),
            PathBuf::from,
        ),
        lease: Duration::from_secs(optional_u64("HEPHAESTUS_OCI_BUILDER_LEASE_SECONDS", 900)?),
        poll_interval: Duration::from_millis(optional_u64(
            "HEPHAESTUS_OCI_BUILDER_POLL_MILLISECONDS",
            1_000,
        )?),
    }))
}

fn optional_u64(name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => Ok(value.parse()?),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(Box::new(error)),
    }
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
            OciImageReference::parse(reference.clone()).map_err(|error| {
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
