//! Administrator-owned local runtime adapters for repository OCI builders.
//!
//! The adapters deliberately accept only opaque durable job identities and
//! paths derived from configured roots. Repository text never becomes a host
//! path, command executable, registry credential, or scanner argument.

use async_trait::async_trait;
use builder_catalog_domain::{BuilderImageReference, OciDigest};
use oci_builder_worker::{
    ClaimedPreparationJob, IsolatedOciBuild, OciOutputPublisher, OciPreparationOutput,
    OciRootfsExporter, OciWorkerError, PreparedSource, RegistryPublisherTokenIssuer,
    RepositoryBuilderPublicationLease, RepositoryBuilderPublicationStore, SourceCheckoutProvider,
    local_image_name,
};
use registry_domain::{
    OciDescriptor, OciMediaType, PublicationIntent, PublicationState, SupplyChainReferrerKind,
};
use registry_publisher::{
    CommandRunner, ControlledOciPublisher, PublicationEvidenceFiles, PublicationMaterial,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::process::Command;

const TRUSTED_SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Explicit absolute binaries and private roots used by the local OCI worker.
#[derive(Debug, Clone)]
pub struct LocalOciRuntimeConfig {
    /// Root containing canonical bare repositories named `<uuid>.git`.
    pub repository_root: PathBuf,
    /// Private transient directory for exact source checkouts.
    pub checkout_root: PathBuf,
    /// Administrator-owned OCI layouts for approved catalog base digests.
    pub base_layouts: BTreeMap<String, PathBuf>,
    /// Private immutable OCI layout output directory.
    pub output_root: PathBuf,
    /// Absolute trusted Git executable.
    pub git_binary: PathBuf,
    /// Absolute trusted Tar executable used only to unpack Git's exact archive.
    pub tar_binary: PathBuf,
    /// Absolute trusted Buildah executable.
    pub buildah_binary: PathBuf,
    /// Absolute trusted Trivy executable.
    pub trivy_binary: PathBuf,
    /// Absolute trusted Umoci executable.
    pub umoci_binary: PathBuf,
    /// Administrator-owned local Buildah image-tag prefix.
    pub buildah_output_prefix: String,
}

/// Trusted tools used only after the networkless Buildah phase.
#[derive(Debug, Clone)]
pub struct ForgeZotPublicationConfig {
    /// Absolute trusted Syft executable used to produce SPDX SBOMs.
    pub syft_binary: PathBuf,
    /// Absolute, administrator-owned Syft configuration.
    pub syft_config: PathBuf,
}

impl ForgeZotPublicationConfig {
    /// Validates and canonicalizes trusted Syft tooling.
    ///
    /// # Errors
    ///
    /// Returns an error when either path is relative, missing, or symbolic.
    pub fn initialize(mut self) -> Result<Self, OciWorkerError> {
        validate_executable(&self.syft_binary)?;
        self.syft_binary =
            fs::canonicalize(&self.syft_binary).map_err(OciWorkerError::Filesystem)?;
        self.syft_config = canonical_regular_file(&self.syft_config)?;
        Ok(self)
    }
}

/// Local implementation of exact checkout, scan/publication, and rootfs export.
#[derive(Clone)]
pub struct LocalOciRuntime {
    config: Arc<LocalOciRuntimeConfig>,
}

impl LocalOciRuntime {
    /// Validates all administrator-controlled binaries and roots.
    ///
    /// # Errors
    ///
    /// Returns an error for a relative, symlinked, or missing configured path.
    pub fn initialize(mut config: LocalOciRuntimeConfig) -> Result<Self, OciWorkerError> {
        validate_executable(&config.git_binary)?;
        validate_executable(&config.tar_binary)?;
        validate_executable(&config.buildah_binary)?;
        validate_executable(&config.trivy_binary)?;
        validate_executable(&config.umoci_binary)?;
        config.repository_root = canonical_directory(&config.repository_root)?;
        config.checkout_root = initialize_directory(&config.checkout_root)?;
        config.output_root = initialize_directory(&config.output_root)?;
        if config.checkout_root.starts_with(&config.repository_root)
            || config.output_root.starts_with(&config.repository_root)
            || config.checkout_root.starts_with(&config.output_root)
            || config.output_root.starts_with(&config.checkout_root)
            || config.buildah_output_prefix.trim().is_empty()
            || config.buildah_output_prefix.len() > 160
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        config.base_layouts = config
            .base_layouts
            .into_iter()
            .map(|(reference, path)| {
                BuilderImageReference::parse(reference.clone())
                    .map_err(|_| OciWorkerError::InvalidConfiguration)?;
                Ok((reference, canonical_directory(&path)?))
            })
            .collect::<Result<_, OciWorkerError>>()?;
        Ok(Self {
            config: Arc::new(config),
        })
    }

    /// Returns the exact local Buildah image name for a job's immutable builder.
    #[must_use]
    pub fn buildah_image_name(&self, request: &IsolatedOciBuild) -> String {
        local_image_name(&self.config.buildah_output_prefix, request.builder_id)
    }

    async fn command_success(
        &self,
        binary: &Path,
        arguments: Vec<OsString>,
    ) -> Result<(), OciWorkerError> {
        let status = Command::new(binary)
            .env_clear()
            // OCI tools may invoke administrator-installed helpers. Keep the
            // path fixed instead of inheriting caller-controlled state.
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .args(&arguments)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        status
            .success()
            .then_some(())
            .ok_or(OciWorkerError::BuildFailed)
    }

    fn layout_directory(&self, request: &IsolatedOciBuild) -> PathBuf {
        self.config
            .output_root
            .join(request.builder_id.as_uuid().to_string())
    }

    async fn publish_layout(
        &self,
        request: &IsolatedOciBuild,
        image_name: &str,
    ) -> Result<PathBuf, OciWorkerError> {
        let layout = self.layout_directory(request);
        if layout.exists() {
            fs::remove_dir_all(&layout).map_err(OciWorkerError::Filesystem)?;
        }
        fs::create_dir_all(&layout).map_err(OciWorkerError::Filesystem)?;
        self.command_success(
            &self.config.buildah_binary,
            vec![
                OsString::from("push"),
                OsString::from("--format"),
                OsString::from("oci"),
                OsString::from(image_name),
                OsString::from(format!("oci:{}:latest", layout.display())),
            ],
        )
        .await?;
        normalize_layout_to_single_index(&layout)
    }

    async fn scan_layout(
        &self,
        request: &IsolatedOciBuild,
        image_name: &str,
        layout: &Path,
    ) -> Result<PathBuf, OciWorkerError> {
        let scan = layout.join("scan.json");
        let archive = self
            .config
            .output_root
            .join(format!("{}.oci.tar", request.builder_id.as_uuid()));
        if archive.exists() {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        self.command_success(
            &self.config.buildah_binary,
            vec![
                OsString::from("push"),
                OsString::from("--format"),
                OsString::from("oci"),
                OsString::from(image_name),
                OsString::from(format!("oci-archive:{}:latest", archive.display())),
            ],
        )
        .await?;
        let scan_result = self
            .command_success(
                &self.config.trivy_binary,
                vec![
                    OsString::from("image"),
                    OsString::from("--input"),
                    archive.clone().into_os_string(),
                    OsString::from("--offline-scan"),
                    OsString::from("--exit-code"),
                    OsString::from("1"),
                    OsString::from("--severity"),
                    OsString::from("CRITICAL,HIGH"),
                    OsString::from("--format"),
                    OsString::from("json"),
                    OsString::from("--output"),
                    scan.as_os_str().to_os_string(),
                ],
            )
            .await;
        fs::remove_file(&archive).map_err(OciWorkerError::Filesystem)?;
        scan_result?;
        if !scan.is_file()
            || fs::symlink_metadata(&scan)
                .map_err(OciWorkerError::Filesystem)?
                .file_type()
                .is_symlink()
        {
            return Err(OciWorkerError::InvalidOutput);
        }
        Ok(scan)
    }

    async fn sbom_layout(
        &self,
        layout: &Path,
        tooling: &ForgeZotPublicationConfig,
    ) -> Result<PathBuf, OciWorkerError> {
        let sbom = layout.join("sbom.spdx.json");
        self.command_success(
            &tooling.syft_binary,
            vec![
                OsString::from("scan"),
                OsString::from(format!("oci-dir:{}", layout.display())),
                OsString::from("--config"),
                tooling.syft_config.as_os_str().to_owned(),
                OsString::from("--output"),
                OsString::from(format!("spdx-json={}", sbom.display())),
                OsString::from("--quiet"),
            ],
        )
        .await?;
        trusted_output_file(layout, &sbom)
    }

    async fn publication_material(
        &self,
        request: &IsolatedOciBuild,
        tooling: &ForgeZotPublicationConfig,
    ) -> Result<PreparedPublicationMaterial, OciWorkerError> {
        let image_name = self.buildah_image_name(request);
        let layout = self.publish_layout(request, &image_name).await?;
        let expected_manifest = layout_index_descriptor(&layout)?;
        let scan = self.scan_layout(request, &image_name, &layout).await?;
        let sbom = self.sbom_layout(&layout, tooling).await?;
        let provenance = layout.join("provenance.json");
        let provenance_document = ProvenanceDocument {
            version: 1,
            project_id: request.project_id,
            builder_id: request.builder_id.as_uuid(),
            base_reference: request.base_reference.as_str(),
            manifest_digest: expected_manifest.digest().as_str(),
        };
        fs::write(
            &provenance,
            serde_json::to_vec(&provenance_document).map_err(OciWorkerError::Serialization)?,
        )
        .map_err(OciWorkerError::Filesystem)?;
        let provenance = trusted_output_file(&layout, &provenance)?;
        Ok(PreparedPublicationMaterial {
            expected_manifest,
            material: PublicationMaterial {
                layout: layout.clone(),
                evidence: PublicationEvidenceFiles {
                    sbom,
                    provenance,
                    scan,
                    signature: None,
                },
            },
            layout,
        })
    }
}

#[async_trait]
impl SourceCheckoutProvider for LocalOciRuntime {
    async fn checkout(
        &self,
        job: &ClaimedPreparationJob,
    ) -> Result<PreparedSource, OciWorkerError> {
        let repository = self
            .config
            .repository_root
            .join(format!("{}.git", job.repository_id));
        let repository = canonical_directory(&repository)?;
        if repository.parent() != Some(self.config.repository_root.as_path()) {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        let checkout = self.config.checkout_root.join(job.id.to_string());
        if checkout.exists() {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        fs::create_dir(&checkout).map_err(OciWorkerError::Filesystem)?;
        let archive = checkout.join("source.tar");
        let archive_status = Command::new(&self.config.git_binary)
            .env_clear()
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                fs::File::create(&archive).map_err(OciWorkerError::Filesystem)?,
            ))
            .stderr(Stdio::null())
            .arg("--git-dir")
            .arg(&repository)
            .arg("archive")
            .arg("--format=tar")
            .arg(&job.source_revision)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        if !archive_status.success() {
            let _ignored = fs::remove_dir_all(&checkout);
            return Err(OciWorkerError::BuildFailed);
        }
        let source = checkout.join("source");
        fs::create_dir(&source).map_err(OciWorkerError::Filesystem)?;
        let tar_status = Command::new(&self.config.tar_binary)
            .env_clear()
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("--no-same-owner")
            .arg("--no-same-permissions")
            .arg("--extract")
            .arg("--file")
            .arg(&archive)
            .arg("--directory")
            .arg(&source)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        let _ignored = fs::remove_file(&archive);
        if !tar_status.success() {
            let _ignored = fs::remove_dir_all(&checkout);
            return Err(OciWorkerError::BuildFailed);
        }
        let source = canonical_directory(&source)?;
        let base_oci_layout = self
            .config
            .base_layouts
            .get(job.base_reference.as_str())
            .cloned()
            .ok_or(OciWorkerError::InvalidConfiguration)?;
        Ok(PreparedSource {
            checkout_root: source,
            base_oci_layout,
        })
    }

    async fn cleanup(&self, source: &PreparedSource) -> Result<(), OciWorkerError> {
        let checkout = source
            .checkout_root
            .parent()
            .ok_or(OciWorkerError::UnsafeSourcePath)?;
        let checkout = fs::canonicalize(checkout).map_err(OciWorkerError::Filesystem)?;
        if checkout.parent() != Some(self.config.checkout_root.as_path()) {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        fs::remove_dir_all(checkout).map_err(OciWorkerError::Filesystem)
    }
}

/// Local OCI artifacts ready for the forge-controlled Zot publication step.
struct PreparedPublicationMaterial {
    expected_manifest: OciDescriptor,
    material: PublicationMaterial,
    layout: PathBuf,
}

/// Combines local output processing with the forge's durable Zot publication protocol.
///
/// Buildah receives no registry token: the token is
/// acquired only after the local OCI index, scan, SBOM, and provenance exist.
pub struct ForgeZotOciPublisher<S, T, R> {
    runtime: LocalOciRuntime,
    tooling: ForgeZotPublicationConfig,
    publications: S,
    tokens: T,
    publisher: ControlledOciPublisher<R>,
}

impl<S, T, R> ForgeZotOciPublisher<S, T, R> {
    /// Creates the repository-builder Zot publication adapter.
    #[must_use]
    pub const fn new(
        runtime: LocalOciRuntime,
        tooling: ForgeZotPublicationConfig,
        publications: S,
        tokens: T,
        publisher: ControlledOciPublisher<R>,
    ) -> Self {
        Self {
            runtime,
            tooling,
            publications,
            tokens,
            publisher,
        }
    }
}

#[async_trait]
impl<S, T, R> OciOutputPublisher for ForgeZotOciPublisher<S, T, R>
where
    S: RepositoryBuilderPublicationStore,
    T: RegistryPublisherTokenIssuer,
    R: CommandRunner + Send + Sync + 'static,
{
    async fn publish(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<OciPreparationOutput, OciWorkerError> {
        let prepared = self
            .runtime
            .publication_material(request, &self.tooling)
            .await?;
        let lease = self
            .publications
            .begin_repository_builder_publication(
                request.project_id,
                request.builder_id,
                prepared.expected_manifest.clone(),
            )
            .await?;
        let approved = match lease {
            RepositoryBuilderPublicationLease::Approved(intent) => intent,
            RepositoryBuilderPublicationLease::Publish(intent) => {
                let token = match self.tokens.issue_pull_push(&intent).await {
                    Ok(token) => token,
                    Err(error) => {
                        self.publications
                            .retry_repository_builder_publication(intent.id())
                            .await?;
                        return Err(error);
                    }
                };
                // Skopeo and ORAS are synchronous trusted tools. Do not block
                // the async runtime worker: Zot may need the daemon's token
                // endpoint while this publication is in progress.
                let verified = tokio::task::block_in_place(|| {
                    self.publisher
                        .publish(&intent, &prepared.material, token.token())
                })
                .map_err(|_| OciWorkerError::RegistryPublication);
                let verification = match verified {
                    Ok(verification) => verification,
                    Err(error) => {
                        self.publications
                            .retry_repository_builder_publication(intent.id())
                            .await?;
                        return Err(error);
                    }
                };
                self.publications
                    .record_verified_and_approve(intent.id(), verification)
                    .await?
            }
        };
        zot_confirmed_output(&approved, prepared.layout)
    }
}

/// The local runtime cannot publish repository-builder output by itself.
///
/// This compatibility implementation deliberately fails closed until the
/// daemon composes [`ForgeZotOciPublisher`] with durable registry and token
/// ports. It replaces the former fabricated registry-like output path.
#[async_trait]
impl OciOutputPublisher for LocalOciRuntime {
    async fn publish(
        &self,
        _request: &IsolatedOciBuild,
    ) -> Result<OciPreparationOutput, OciWorkerError> {
        Err(OciWorkerError::RegistryPublication)
    }
}

#[async_trait]
impl OciRootfsExporter for LocalOciRuntime {
    async fn export_rootfs(
        &self,
        _image_reference: &BuilderImageReference,
        local_oci_layout: &Path,
        destination: &Path,
    ) -> Result<(), OciWorkerError> {
        let bundle = destination.join(".umoci-bundle");
        self.command_success(
            &self.config.umoci_binary,
            vec![
                OsString::from("unpack"),
                OsString::from("--rootless"),
                OsString::from("--image"),
                OsString::from(format!("{}:latest", local_oci_layout.display())),
                bundle.as_os_str().to_os_string(),
            ],
        )
        .await?;
        let rootfs = bundle.join("rootfs");
        let entries = fs::read_dir(&rootfs).map_err(OciWorkerError::Filesystem)?;
        for entry in entries {
            let entry = entry.map_err(OciWorkerError::Filesystem)?;
            fs::rename(entry.path(), destination.join(entry.file_name()))
                .map_err(OciWorkerError::Filesystem)?;
        }
        fs::remove_dir_all(bundle).map_err(OciWorkerError::Filesystem)
    }
}

#[derive(Serialize)]
struct ProvenanceDocument<'a> {
    version: u32,
    project_id: uuid::Uuid,
    builder_id: uuid::Uuid,
    base_reference: &'a str,
    manifest_digest: &'a str,
}

fn validate_executable(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if !path.is_absolute()
        || metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(())
}

fn canonical_regular_file(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    fs::canonicalize(path).map_err(OciWorkerError::Filesystem)
}

fn initialize_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    fs::create_dir_all(path).map_err(OciWorkerError::Filesystem)?;
    canonical_directory(path)
}

fn canonical_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    let path = fs::canonicalize(path).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(path)
}

fn normalize_layout_to_single_index(layout: &Path) -> Result<PathBuf, OciWorkerError> {
    let index_path = layout.join("index.json");
    let source: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).map_err(OciWorkerError::Filesystem)?)
            .map_err(OciWorkerError::Serialization)?;
    let manifests = source
        .get("manifests")
        .and_then(serde_json::Value::as_array)
        .filter(|manifests| manifests.len() == 1)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let mut manifest = manifests[0].clone();
    let digest = manifest
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let digest = OciDigest::parse(digest.to_owned()).map_err(|_| OciWorkerError::InvalidOutput)?;
    let size = manifest
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .filter(|size| *size > 0)
        .ok_or(OciWorkerError::InvalidOutput)?;
    manifest
        .get("mediaType")
        .and_then(serde_json::Value::as_str)
        .filter(|media_type| *media_type == OciMediaType::IMAGE_MANIFEST)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let blob = layout.join("blobs").join("sha256").join(
        digest
            .as_str()
            .strip_prefix("sha256:")
            .ok_or(OciWorkerError::InvalidOutput)?,
    );
    let blob_bytes = fs::read(&blob).map_err(OciWorkerError::Filesystem)?;
    if u64::try_from(blob_bytes.len()).map_err(|_| OciWorkerError::InvalidOutput)? != size
        || format!("sha256:{:x}", Sha256::digest(&blob_bytes)) != digest.as_str()
    {
        return Err(OciWorkerError::InvalidOutput);
    }
    let object = manifest
        .as_object_mut()
        .ok_or(OciWorkerError::InvalidOutput)?;
    object.insert(
        String::from("platform"),
        serde_json::json!({ "os": "linux", "architecture": "amd64" }),
    );
    let index_bytes = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OciMediaType::IMAGE_INDEX,
        "manifests": [manifest],
    }))
    .map_err(OciWorkerError::Serialization)?;
    let index_digest = format!("sha256:{:x}", Sha256::digest(&index_bytes));
    let index_blob = layout.join("blobs").join("sha256").join(
        index_digest
            .strip_prefix("sha256:")
            .ok_or(OciWorkerError::InvalidOutput)?,
    );
    fs::write(&index_blob, &index_bytes).map_err(OciWorkerError::Filesystem)?;
    let reference_tag = format!("heph-{}", index_digest.replace(':', "-"));
    fs::write(
        &index_path,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": OciMediaType::IMAGE_INDEX,
                "digest": index_digest,
                "size": index_bytes.len(),
                "annotations": { "org.opencontainers.image.ref.name": reference_tag },
            }],
        }))
        .map_err(OciWorkerError::Serialization)?,
    )
    .map_err(OciWorkerError::Filesystem)?;
    Ok(layout.to_path_buf())
}

fn layout_index_descriptor(layout: &Path) -> Result<OciDescriptor, OciWorkerError> {
    let index: serde_json::Value = serde_json::from_slice(
        &fs::read(layout.join("index.json")).map_err(OciWorkerError::Filesystem)?,
    )
    .map_err(OciWorkerError::Serialization)?;
    let descriptor = index
        .get("manifests")
        .and_then(serde_json::Value::as_array)
        .and_then(|manifests| manifests.first())
        .ok_or(OciWorkerError::InvalidOutput)?;
    let digest = descriptor
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or(OciWorkerError::InvalidOutput)?;
    let size = descriptor
        .get("size")
        .and_then(serde_json::Value::as_u64)
        .ok_or(OciWorkerError::InvalidOutput)?;
    OciDescriptor::new(
        registry_domain::Sha256Digest::parse(digest.to_owned())
            .map_err(|_| OciWorkerError::InvalidOutput)?,
        size,
        OciMediaType::parse(OciMediaType::IMAGE_INDEX)
            .map_err(|_| OciWorkerError::InvalidOutput)?,
    )
    .map_err(|_| OciWorkerError::InvalidOutput)
}

fn trusted_output_file(root: &Path, file: &Path) -> Result<PathBuf, OciWorkerError> {
    let file = fs::canonicalize(file).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&file).map_err(OciWorkerError::Filesystem)?;
    (file.starts_with(root) && metadata.is_file() && !metadata.file_type().is_symlink())
        .then_some(file)
        .ok_or(OciWorkerError::InvalidOutput)
}

fn zot_confirmed_output(
    intent: &PublicationIntent,
    local_oci_layout: PathBuf,
) -> Result<OciPreparationOutput, OciWorkerError> {
    if intent.state() != PublicationState::Approved {
        return Err(OciWorkerError::RegistryPublication);
    }
    let reference = intent
        .approved_reference()
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    let verification = intent
        .verification()
        .ok_or(OciWorkerError::RegistryPublication)?;
    let evidence_reference = |kind| -> Result<String, OciWorkerError> {
        let referrer = verification
            .evidence()
            .referrers()
            .iter()
            .find(|referrer| referrer.kind() == kind)
            .ok_or(OciWorkerError::RegistryPublication)?;
        Ok(format!(
            "{}/{}@{}",
            reference.authority(),
            reference.namespace(),
            referrer.descriptor().digest()
        ))
    };
    let image_reference = BuilderImageReference::parse(reference.to_string())
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    let image_digest = OciDigest::parse(reference.digest().as_str().to_owned())
        .map_err(|_| OciWorkerError::RegistryPublication)?;
    Ok(OciPreparationOutput {
        image_reference,
        image_digest,
        attestation_reference: evidence_reference(SupplyChainReferrerKind::Provenance)?,
        sbom_reference: Some(evidence_reference(SupplyChainReferrerKind::Sbom)?),
        scan_reference: evidence_reference(SupplyChainReferrerKind::Scan)?,
        local_oci_layout,
    })
}

#[cfg(test)]
mod tests {
    use super::{LocalOciRuntime, LocalOciRuntimeConfig, zot_confirmed_output};
    use builder_catalog_domain::ProjectBuilderId;
    use oci_builder_worker::{PreparedSource, SourceCheckoutProvider};
    use registry_domain::{
        ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType,
        PlatformDescriptor, PolicyVersion, PublicationIntent, PublicationIntentId,
        RegistryAuthority, RegistryNamespace, Sha256Digest, SupplyChainEvidence, SupplyChainPolicy,
        SupplyChainReferrer, SupplyChainReferrerKind, VerifiedPublication,
    };
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    fn executable(root: &std::path::Path, name: &str) -> std::path::PathBuf {
        let path = root.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write test executable");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("set test executable permissions");
        path
    }

    fn configuration(root: &std::path::Path) -> LocalOciRuntimeConfig {
        let base = root.join("base");
        fs::create_dir(&base).expect("create base layout directory");
        let binary = executable(root, "trusted-tool");
        LocalOciRuntimeConfig {
            repository_root: {
                let path = root.join("repositories");
                fs::create_dir(&path).expect("create repository root");
                path
            },
            checkout_root: root.join("checkouts"),
            base_layouts: BTreeMap::from([(
                String::from(
                    "registry.example/heph-base@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                base,
            )]),
            output_root: root.join("outputs"),
            git_binary: binary.clone(),
            tar_binary: binary.clone(),
            buildah_binary: binary.clone(),
            trivy_binary: binary.clone(),
            umoci_binary: binary,
            buildah_output_prefix: String::from("heph-builder"),
        }
    }

    fn descriptor(character: char, media_type: &str) -> OciDescriptor {
        OciDescriptor::new(
            Sha256Digest::parse(format!("sha256:{}", character.to_string().repeat(64)))
                .expect("digest"),
            42,
            OciMediaType::parse(media_type).expect("media type"),
        )
        .expect("descriptor")
    }

    fn approved_intent() -> PublicationIntent {
        let project_id = Uuid::from_u128(1);
        let builder_id = ProjectBuilderId::from_uuid(Uuid::from_u128(2));
        let namespace = RegistryNamespace::parse(format!(
            "projects/{project_id}/repository-builders/{builder_id}"
        ))
        .expect("repository builder namespace");
        let claim = NamespaceClaim::new(namespace.owner().clone());
        let manifest = descriptor('a', OciMediaType::IMAGE_INDEX);
        let reference = ImmutableManifestReference::new(
            RegistryAuthority::parse("registry.example").expect("authority"),
            claim.namespace().clone(),
            manifest.digest().clone(),
        );
        let evidence = SupplyChainEvidence::new(
            manifest.digest().clone(),
            [
                (SupplyChainReferrerKind::Sbom, 'b'),
                (SupplyChainReferrerKind::Provenance, 'c'),
                (SupplyChainReferrerKind::Scan, 'd'),
            ]
            .into_iter()
            .map(|(kind, character)| {
                SupplyChainReferrer::new(
                    kind,
                    manifest.digest().clone(),
                    descriptor(character, "application/vnd.oci.image.manifest.v1+json"),
                    OciMediaType::parse(match kind {
                        SupplyChainReferrerKind::Sbom => "application/spdx+json",
                        SupplyChainReferrerKind::Provenance => "application/vnd.in-toto+json",
                        SupplyChainReferrerKind::Scan => {
                            "application/vnd.hephaestus.vulnerability-scan.v1+json"
                        }
                        SupplyChainReferrerKind::Signature => {
                            "application/vnd.dev.cosign.simplesigning.v1+json"
                        }
                    })
                    .expect("artifact type"),
                )
            })
            .collect(),
        )
        .expect("evidence");
        let verification = VerifiedPublication::new(
            &reference,
            manifest.clone(),
            vec![
                PlatformDescriptor::new(
                    descriptor('e', OciMediaType::IMAGE_MANIFEST),
                    "linux",
                    "amd64",
                    None,
                )
                .expect("platform"),
            ],
            evidence,
        )
        .expect("verification");
        PublicationIntent::new(
            PublicationIntentId::from_uuid(Uuid::from_u128(3)),
            claim,
            reference,
            manifest,
            PolicyVersion::parse("builder-v1").expect("policy version"),
            SupplyChainPolicy::without_signature(),
        )
        .expect("intent")
        .begin_publishing()
        .expect("publishing")
        .record_verified(verification)
        .expect("verified")
        .approve()
        .expect("approved")
    }

    #[test]
    fn initialization_rejects_overlapping_private_roots() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let mut config = configuration(temporary.path());
        config.output_root = config.checkout_root.clone();
        assert!(LocalOciRuntime::initialize(config).is_err());
    }

    #[test]
    fn cleanup_removes_only_the_job_checkout() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let runtime = LocalOciRuntime::initialize(configuration(temporary.path()))
            .expect("safe local runtime");
        let checkout = temporary.path().join("checkouts/job/source");
        fs::create_dir_all(&checkout).expect("create source checkout");
        let source = PreparedSource {
            checkout_root: fs::canonicalize(&checkout).expect("canonical source checkout"),
            base_oci_layout: temporary.path().join("base"),
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio runtime")
            .block_on(runtime.cleanup(&source))
            .expect("remove job checkout");
        assert!(!temporary.path().join("checkouts/job").exists());
        assert!(temporary.path().join("checkouts").exists());
    }

    #[test]
    fn only_a_verified_and_approved_zot_intent_becomes_durable_builder_output() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let layout = temporary.path().join("layout");
        fs::create_dir(&layout).expect("layout");
        let output =
            zot_confirmed_output(&approved_intent(), layout.clone()).expect("Zot-confirmed output");
        assert_eq!(
            output.image_reference.as_str(),
            "registry.example/projects/00000000-0000-0000-0000-000000000001/repository-builders/00000000-0000-0000-0000-000000000002@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(output.attestation_reference.ends_with(&"c".repeat(64)));
        assert!(
            output
                .sbom_reference
                .expect("SBOM")
                .ends_with(&"b".repeat(64))
        );
        assert!(output.scan_reference.ends_with(&"d".repeat(64)));
        assert_eq!(output.local_oci_layout, layout);
    }
}
