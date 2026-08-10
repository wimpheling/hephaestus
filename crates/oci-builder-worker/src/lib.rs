//! Isolated, durable OCI production and rootfs materialization.
//!
//! This crate has no database or RPC dependency. It consumes claimed durable
//! jobs, materializes one exact source tree, invokes a rootless OCI image
//! with no network or credentials, and commits outcomes through its job port.

use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageId, OciImageReference};
use registry_domain::{OciDescriptor, PublicationIntent, PublicationIntentId, VerifiedPublication};
use registry_token::IssuedToken;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;
use uuid::Uuid;

const TRUSTED_SYSTEM_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// A validated repository-relative Dockerfile or OCI build-context path.
///
/// This is repository-source metadata, not image-catalog metadata: an image
/// remains an image after it has been produced.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryOciImageSourcePath(String);

impl RepositoryOciImageSourcePath {
    /// Parses a bounded, traversal-free repository-relative POSIX path.
    ///
    /// # Errors
    ///
    /// Returns [`OciWorkerError::UnsafeSourcePath`] for a malformed path.
    pub fn parse(value: impl Into<String>) -> Result<Self, OciWorkerError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 1024
            && value == value.trim()
            && !value.starts_with('/')
            && !value.contains('\\')
            && !value.bytes().any(|byte| byte.is_ascii_control())
            && (value == "."
                || value.split('/').all(|component| {
                    !component.is_empty() && component != "." && component != ".."
                }));
        valid
            .then_some(Self(value))
            .ok_or(OciWorkerError::UnsafeSourcePath)
    }

    /// Returns the validated path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Immutable provenance for one repository-produced OCI image.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RepositoryOciImageProvenance {
    /// Exact Git revision used as the source.
    pub source_revision: String,
    /// Digest of the exact source context.
    pub context_digest: OciDigest,
    /// Immutable build-attestation reference.
    pub attestation_reference: String,
    /// Optional immutable SBOM reference.
    pub sbom_reference: Option<String>,
}

impl RepositoryOciImageProvenance {
    /// Validates immutable repository-image provenance.
    ///
    /// # Errors
    ///
    /// Returns [`OciWorkerError::InvalidOutput`] for malformed provenance.
    pub fn validate(&self) -> Result<(), OciWorkerError> {
        let valid_revision = matches!(self.source_revision.len(), 40 | 64)
            && self
                .source_revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
        let valid_reference = |reference: &str| {
            !reference.trim().is_empty()
                && reference.len() <= 2048
                && !reference.bytes().any(|byte| byte.is_ascii_control())
        };
        (valid_revision
            && valid_reference(&self.attestation_reference)
            && self.sbom_reference.as_deref().is_none_or(valid_reference))
        .then_some(())
        .ok_or(OciWorkerError::InvalidOutput)
    }
}

/// A claimed durable production request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedProductionJob {
    /// Durable job identifier.
    pub id: Uuid,
    /// Opaque durable project identity that owns this image.
    pub project_id: Uuid,
    /// Owning image definition.
    pub image_id: OciImageId,
    /// Owning repository.
    pub repository_id: Uuid,
    /// Immutable Git revision.
    pub source_revision: String,
    /// Expected digest of the build context.
    pub context_digest: OciDigest,
    /// Dockerfile path within the checkout.
    pub dockerfile_path: RepositoryOciImageSourcePath,
    /// Context path within the checkout.
    pub context_path: RepositoryOciImageSourcePath,
    /// Resolved approved platform base.
    pub base_reference: OciImageReference,
}

/// One prepared source checkout supplied by the trusted Git materializer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSource {
    /// Absolute, private source checkout path.
    pub checkout_root: PathBuf,
    /// Absolute OCI-layout directory for the approved base image.
    pub base_oci_layout: PathBuf,
}

/// Arguments exposed to an isolated OCI build engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedOciBuild {
    /// Repository OCI image definition being prepared.
    pub image_id: OciImageId,
    /// Opaque durable project identity that owns this image.
    pub project_id: Uuid,
    /// Absolute canonical Dockerfile path.
    pub dockerfile: PathBuf,
    /// Absolute canonical context path.
    pub context: PathBuf,
    /// Local immutable OCI layout bound as the `heph-base` build context.
    pub base_oci_layout: PathBuf,
    /// Expected catalog base image reference for attestation.
    pub base_reference: OciImageReference,
    /// Whether the build sandbox permits guest network access.
    pub network_disabled: bool,
    /// Whether credentials, secrets, and host sockets are available.
    pub ambient_credentials_disabled: bool,
}

/// Immutable output and required supply-chain results from OCI production.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciImageProductionOutput {
    /// Published immutable output reference.
    pub image_reference: OciImageReference,
    /// Digest copied from `image_reference`.
    pub image_digest: OciDigest,
    /// Build attestation reference.
    pub attestation_reference: String,
    /// Optional software bill of materials reference.
    pub sbom_reference: Option<String>,
    /// Required successful vulnerability/allow-list scan result reference.
    pub scan_reference: String,
    /// Local immutable OCI layout used solely by the materializer.
    pub local_oci_layout: PathBuf,
}

/// Durable materialization work claimed by a daemon-specific worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedMaterializationJob {
    /// Durable materialization job identity.
    pub id: Uuid,
    /// Immutable output to export.
    pub image_reference: OciImageReference,
}

/// One root filesystem that may be placed in a daemon manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaterializedRoot {
    /// Immutable output reference used as the manifest key.
    pub image_reference: OciImageReference,
    /// Absolute, canonical root filesystem directory.
    pub root_path: PathBuf,
}

/// PostgreSQL or equivalent durable worker boundary.
#[async_trait]
pub trait OciImageProductionJobStore: Send + Sync + 'static {
    /// Claims one queued or expired production job.
    async fn claim_production(
        &self,
        worker_name: &str,
        lease: Duration,
    ) -> Result<Option<ClaimedProductionJob>, OciWorkerStoreError>;

    /// Records verified production output and queues materialization.
    async fn complete_production(
        &self,
        job_id: Uuid,
        materialization_worker_name: &str,
        output: &OciImageProductionOutput,
        provenance: RepositoryOciImageProvenance,
    ) -> Result<(), OciWorkerStoreError>;

    /// Records a non-sensitive, bounded production failure.
    async fn fail_production(&self, job_id: Uuid, reason: &str) -> Result<(), OciWorkerStoreError>;

    /// Claims one materialization job for this daemon worker.
    async fn claim_materialization(
        &self,
        worker_name: &str,
        lease: Duration,
    ) -> Result<Option<ClaimedMaterializationJob>, OciWorkerStoreError>;

    /// Records an atomically installed root filesystem.
    async fn complete_materialization(
        &self,
        job_id: Uuid,
        root_path: &Path,
    ) -> Result<(), OciWorkerStoreError>;

    /// Records a non-sensitive, bounded materialization failure.
    async fn fail_materialization(
        &self,
        job_id: Uuid,
        reason: &str,
    ) -> Result<(), OciWorkerStoreError>;

    /// Lists roots that have completed materialization for one daemon worker.
    async fn materialized_roots(
        &self,
        worker_name: &str,
    ) -> Result<Vec<MaterializedRoot>, OciWorkerStoreError>;
}

/// Trusted source materialization boundary.
#[async_trait]
pub trait SourceCheckoutProvider: Send + Sync + 'static {
    /// Materializes only the exact revision and approved base requested by a job.
    async fn checkout(&self, job: &ClaimedProductionJob) -> Result<PreparedSource, OciWorkerError>;

    /// Removes a checkout after its OCI build attempt has reached a terminal
    /// worker outcome. Implementations may retain no source-controlled files.
    async fn cleanup(&self, _source: &PreparedSource) -> Result<(), OciWorkerError> {
        Ok(())
    }
}

/// Rootless OCI engine boundary.
#[async_trait]
pub trait OciBuildEngine: Send + Sync + 'static {
    /// Builds an already policy-validated source tree with isolated inputs.
    async fn build(
        &self,
        request: IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError>;
}

/// Platform-owned publication, scanning, and attestation boundary following a
/// successful isolated OCI build.
#[async_trait]
pub trait OciOutputPublisher: Send + Sync + 'static {
    /// Scans, records provenance, and returns the immutable published output.
    async fn publish(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError>;
}

/// The durable publication state returned after a repository-image intent
/// has been created or resumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryOciImagePublicationLease {
    /// Zot publication must be attempted with the enclosed exact intent.
    Publish(PublicationIntent),
    /// A prior attempt already verified and approved the exact immutable output.
    Approved(PublicationIntent),
}

/// Durable registry control-plane boundary for repository-image outputs.
///
/// The local build runtime never derives registry namespaces or creates a
/// reference itself. It supplies only opaque owner identifiers and the exact
/// OCI index descriptor produced from its trusted local layout.
#[async_trait]
pub trait RepositoryOciImagePublicationStore: Send + Sync + 'static {
    /// Creates or resumes the exact repository-image publication intent.
    async fn begin_repository_image_publication(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
        expected_manifest: OciDescriptor,
    ) -> Result<RepositoryOciImagePublicationLease, OciWorkerError>;

    /// Records Zot read-back evidence and commits the matching intent as approved.
    async fn record_verified_and_approve(
        &self,
        intent_id: PublicationIntentId,
        verification: VerifiedPublication,
    ) -> Result<PublicationIntent, OciWorkerError>;

    /// Returns an interrupted publication to a retryable pending state.
    async fn retry_repository_image_publication(
        &self,
        intent_id: PublicationIntentId,
    ) -> Result<(), OciWorkerError>;
}

/// Narrow workload-token boundary for one exact publication intent.
///
/// Implementations must issue only short-lived `pull,push` credentials for
/// the supplied intent namespace. The token is deliberately obtained after
/// Buildah completes and is never made available to the build sandbox.
#[async_trait]
pub trait RegistryPublisherTokenIssuer: Send + Sync + 'static {
    /// Issues a short-lived bearer token bound to this exact Zot namespace.
    async fn issue_pull_push(
        &self,
        intent: &PublicationIntent,
    ) -> Result<IssuedToken, OciWorkerError>;
}

/// OCI layout-to-rootfs exporter boundary.
#[async_trait]
pub trait OciRootfsExporter: Send + Sync + 'static {
    /// Pulls and verifies one immutable registry image into an empty
    /// destination. The durable job contains no caller-controlled local OCI
    /// path, so a rootfs is always a cache of forge registry content.
    async fn export_rootfs(
        &self,
        image_reference: &OciImageReference,
        destination: &Path,
    ) -> Result<(), OciWorkerError>;
}

/// Isolated OCI production worker.
pub struct OciImageProductionWorker<S, C, E> {
    store: S,
    checkout: C,
    engine: E,
    worker_name: String,
    materialization_worker_name: String,
    lease: Duration,
}

impl<S, C, E> OciImageProductionWorker<S, C, E>
where
    S: OciImageProductionJobStore,
    C: SourceCheckoutProvider,
    E: OciBuildEngine,
{
    /// Creates a worker with a stable operator-visible identity.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty identity or zero lease.
    pub fn new(
        store: S,
        checkout: C,
        engine: E,
        worker_name: String,
        materialization_worker_name: String,
        lease: Duration,
    ) -> Result<Self, OciWorkerError> {
        if worker_name.trim().is_empty()
            || worker_name.len() > 200
            || materialization_worker_name.trim().is_empty()
            || materialization_worker_name.len() > 200
            || lease.is_zero()
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        Ok(Self {
            store,
            checkout,
            engine,
            worker_name,
            materialization_worker_name,
            lease,
        })
    }

    /// Processes at most one durable job. Redelivery is harmless because the
    /// store's claim and terminal transitions are compare-and-swap operations.
    ///
    /// # Errors
    ///
    /// Returns a checkout, Dockerfile policy, engine, or durable-store error.
    pub async fn run_once(&self) -> Result<bool, OciWorkerError> {
        let Some(job) = self
            .store
            .claim_production(&self.worker_name, self.lease)
            .await
            .map_err(OciWorkerError::Store)?
        else {
            return Ok(false);
        };
        let result = self.prepare(&job).await;
        match result {
            Ok((output, provenance)) => self
                .store
                .complete_production(
                    job.id,
                    &self.materialization_worker_name,
                    &output,
                    provenance,
                )
                .await
                .map_err(OciWorkerError::Store)?,
            Err(error) => self
                .store
                .fail_production(job.id, &bounded_reason(&error))
                .await
                .map_err(OciWorkerError::Store)?,
        }
        Ok(true)
    }

    async fn prepare(
        &self,
        job: &ClaimedProductionJob,
    ) -> Result<(OciImageProductionOutput, RepositoryOciImageProvenance), OciWorkerError> {
        let source = self.checkout.checkout(job).await?;
        let result = async {
            let request = isolated_request(job, &source)?;
            let output = self.engine.build(request).await?;
            validate_output(job, &output)?;
            let provenance = RepositoryOciImageProvenance {
                source_revision: job.source_revision.clone(),
                context_digest: job.context_digest.clone(),
                attestation_reference: output.attestation_reference.clone(),
                sbom_reference: output.sbom_reference.clone(),
            };
            provenance
                .validate()
                .map_err(|_| OciWorkerError::InvalidOutput)?;
            Ok((output, provenance))
        }
        .await;
        self.checkout.cleanup(&source).await?;
        result
    }
}

/// Daemon-local rootfs materialization worker.
pub struct RootfsMaterializationWorker<S, E> {
    store: S,
    exporter: E,
    worker_name: String,
    rootfs_root: PathBuf,
    lease: Duration,
}

impl<S, E> RootfsMaterializationWorker<S, E>
where
    S: OciImageProductionJobStore,
    E: OciRootfsExporter,
{
    /// Creates a materializer rooted at one private daemon-owned directory.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe roots, identities, or leases.
    pub fn new(
        store: S,
        exporter: E,
        worker_name: String,
        rootfs_root: PathBuf,
        lease: Duration,
    ) -> Result<Self, OciWorkerError> {
        if worker_name.trim().is_empty()
            || worker_name.len() > 200
            || !rootfs_root.is_absolute()
            || lease.is_zero()
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        fs::create_dir_all(&rootfs_root).map_err(OciWorkerError::Filesystem)?;
        let rootfs_root = fs::canonicalize(rootfs_root).map_err(OciWorkerError::Filesystem)?;
        if fs::symlink_metadata(&rootfs_root)
            .map_err(OciWorkerError::Filesystem)?
            .file_type()
            .is_symlink()
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        Ok(Self {
            store,
            exporter,
            worker_name,
            rootfs_root,
            lease,
        })
    }

    /// Materializes at most one claimed job through an empty staging directory.
    ///
    /// # Errors
    ///
    /// Returns an exporter, filesystem-safety, or durable-store error.
    pub async fn run_once(&self) -> Result<bool, OciWorkerError> {
        let Some(job) = self
            .store
            .claim_materialization(&self.worker_name, self.lease)
            .await
            .map_err(OciWorkerError::Store)?
        else {
            return Ok(false);
        };
        let destination = self
            .rootfs_root
            .join(digest_directory_name(&job.image_reference)?);
        let staging = self.rootfs_root.join(format!(".{}.{}", job.id, "staging"));
        let result = self.materialize(&job, &staging, &destination).await;
        match result {
            Ok(()) => self
                .store
                .complete_materialization(job.id, &destination)
                .await
                .map_err(OciWorkerError::Store)?,
            Err(error) => self
                .store
                .fail_materialization(job.id, &bounded_reason(&error))
                .await
                .map_err(OciWorkerError::Store)?,
        }
        Ok(true)
    }

    async fn materialize(
        &self,
        job: &ClaimedMaterializationJob,
        staging: &Path,
        destination: &Path,
    ) -> Result<(), OciWorkerError> {
        if staging.exists() {
            return Err(OciWorkerError::UnsafeMaterializationPath);
        }
        fs::create_dir(staging).map_err(OciWorkerError::Filesystem)?;
        let export = self
            .exporter
            .export_rootfs(&job.image_reference, staging)
            .await;
        if let Err(error) = export {
            let _ = fs::remove_dir_all(staging);
            return Err(error);
        }
        let metadata = fs::symlink_metadata(staging).map_err(OciWorkerError::Filesystem)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            let _ = fs::remove_dir_all(staging);
            return Err(OciWorkerError::UnsafeMaterializationPath);
        }
        if destination.exists() {
            let _ = fs::remove_dir_all(staging);
            return Err(OciWorkerError::UnsafeMaterializationPath);
        }
        fs::rename(staging, destination).map_err(OciWorkerError::Filesystem)
    }

    /// Writes an atomic daemon root manifest using only durable successful
    /// materialization rows. Unprepared or failed digest references cannot be
    /// added through this path.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsafe path, invalid durable root, or failed
    /// filesystem write.
    pub async fn write_manifest(&self, manifest: &Path) -> Result<(), OciWorkerError> {
        if !manifest.is_absolute() || manifest.extension().is_none_or(|value| value != "json") {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let roots = self
            .store
            .materialized_roots(&self.worker_name)
            .await
            .map_err(OciWorkerError::Store)?;
        let mut entries = BTreeMap::new();
        for root in roots {
            let canonical =
                fs::canonicalize(&root.root_path).map_err(OciWorkerError::Filesystem)?;
            if !canonical.starts_with(&self.rootfs_root)
                || fs::symlink_metadata(&canonical)
                    .map_err(OciWorkerError::Filesystem)?
                    .file_type()
                    .is_symlink()
            {
                return Err(OciWorkerError::UnsafeMaterializationPath);
            }
            entries.insert(
                root.image_reference.to_string(),
                RootManifestEntry::Directory { path: canonical },
            );
        }
        let parent = manifest
            .parent()
            .ok_or(OciWorkerError::InvalidConfiguration)?;
        fs::create_dir_all(parent).map_err(OciWorkerError::Filesystem)?;
        let temporary = parent.join(format!(
            ".{}.{}",
            manifest.file_name().unwrap_or_default().to_string_lossy(),
            Uuid::new_v4()
        ));
        let bytes = serde_json::to_vec(&RootManifest {
            version: 1,
            roots: entries,
        })
        .map_err(OciWorkerError::Serialization)?;
        fs::write(&temporary, bytes).map_err(OciWorkerError::Filesystem)?;
        fs::rename(temporary, manifest).map_err(OciWorkerError::Filesystem)
    }
}

#[derive(Serialize)]
struct RootManifest {
    version: u32,
    roots: BTreeMap<String, RootManifestEntry>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RootManifestEntry {
    Directory { path: PathBuf },
}

fn isolated_request(
    job: &ClaimedProductionJob,
    source: &PreparedSource,
) -> Result<IsolatedOciBuild, OciWorkerError> {
    let checkout = canonical_directory(&source.checkout_root)?;
    let dockerfile = safe_child(&checkout, &job.dockerfile_path, false)?;
    let context = safe_child(&checkout, &job.context_path, true)?;
    let base_oci_layout = canonical_directory(&source.base_oci_layout)?;
    validate_tree(&context)?;
    let dockerfile_text = fs::read_to_string(&dockerfile).map_err(OciWorkerError::Filesystem)?;
    DockerfilePolicy::validate(&dockerfile_text)?;
    Ok(IsolatedOciBuild {
        image_id: job.image_id,
        project_id: job.project_id,
        dockerfile,
        context,
        base_oci_layout,
        base_reference: job.base_reference.clone(),
        network_disabled: true,
        ambient_credentials_disabled: true,
    })
}

fn canonical_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    let canonical = fs::canonicalize(path).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&canonical).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    Ok(canonical)
}

fn safe_child(
    root: &Path,
    relative: &RepositoryOciImageSourcePath,
    directory: bool,
) -> Result<PathBuf, OciWorkerError> {
    let canonical =
        fs::canonicalize(root.join(relative.as_str())).map_err(OciWorkerError::Filesystem)?;
    if !canonical.starts_with(root) {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    let metadata = fs::symlink_metadata(&canonical).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || metadata.is_dir() != directory {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    Ok(canonical)
}

fn validate_tree(root: &Path) -> Result<(), OciWorkerError> {
    for entry in fs::read_dir(root).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let metadata = entry.file_type().map_err(OciWorkerError::Filesystem)?;
        if metadata.is_symlink() {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        if metadata.is_dir() {
            validate_tree(&entry.path())?;
        }
    }
    Ok(())
}

fn validate_output(
    job: &ClaimedProductionJob,
    output: &OciImageProductionOutput,
) -> Result<(), OciWorkerError> {
    if output
        .image_reference
        .digest()
        .map_err(|_| OciWorkerError::InvalidOutput)?
        != output.image_digest
        || output.attestation_reference.trim().is_empty()
        || output.attestation_reference.len() > 2048
        || output.scan_reference.trim().is_empty()
        || output.scan_reference.len() > 2048
        || output
            .sbom_reference
            .as_ref()
            .is_some_and(|reference| reference.trim().is_empty() || reference.len() > 2048)
        || job.base_reference.as_str().is_empty()
    {
        return Err(OciWorkerError::InvalidOutput);
    }
    Ok(())
}

fn digest_directory_name(reference: &OciImageReference) -> Result<String, OciWorkerError> {
    let digest = reference
        .digest()
        .map_err(|_| OciWorkerError::InvalidOutput)?;
    Ok(digest.as_str().replace(':', "-"))
}

fn bounded_reason(error: &OciWorkerError) -> String {
    let reason = error.to_string();
    reason.chars().take(2048).collect()
}

/// Dockerfile policy required before invoking any OCI implementation.
pub struct DockerfilePolicy;

impl DockerfilePolicy {
    /// Validates the restricted `heph-base` Dockerfile contract.
    ///
    /// # Errors
    ///
    /// Returns an error for an unapproved base, a remote context source, or a
    /// malformed `FROM` instruction.
    pub fn validate(source: &str) -> Result<(), OciWorkerError> {
        let mut stages = Vec::<String>::new();
        let mut saw_from = false;
        for raw_line in source.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Multi-line instructions need a full Dockerfile parser. Rejecting
            // them is deliberately conservative: otherwise a remote source
            // could be hidden on a continuation line from this policy check.
            if line.ends_with('\\') {
                return Err(OciWorkerError::InvalidDockerfile);
            }
            let mut parts = line.split_whitespace();
            let Some(instruction) = parts.next() else {
                continue;
            };
            if instruction.eq_ignore_ascii_case("from") {
                let tokens: Vec<_> = parts
                    .filter(|part| !part.starts_with("--platform="))
                    .collect();
                let Some(image) = tokens.first() else {
                    return Err(OciWorkerError::InvalidDockerfile);
                };
                let allowed = if saw_from {
                    *image == "scratch" || stages.iter().any(|stage| stage == image)
                } else {
                    *image == "heph-base"
                };
                if !allowed {
                    return Err(OciWorkerError::UnapprovedDockerfileBase);
                }
                if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("as") {
                    let name = tokens[2];
                    if !valid_stage_name(name) {
                        return Err(OciWorkerError::InvalidDockerfile);
                    }
                    stages.push(String::from(name));
                } else if tokens.len() != 1 {
                    return Err(OciWorkerError::InvalidDockerfile);
                }
                saw_from = true;
            } else if instruction.eq_ignore_ascii_case("add")
                || instruction.eq_ignore_ascii_case("copy")
            {
                let tokens: Vec<_> = parts.filter(|part| !part.starts_with("--")).collect();
                let lower = line.to_ascii_lowercase();
                if tokens.iter().any(|token| is_remote_source(token))
                    || lower.contains("http://")
                    || lower.contains("https://")
                    || lower.contains("git://")
                    || lower.contains("ssh://")
                {
                    return Err(OciWorkerError::RemoteDockerfileSource);
                }
            }
        }
        saw_from
            .then_some(())
            .ok_or(OciWorkerError::InvalidDockerfile)
    }
}

fn valid_stage_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn is_remote_source(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("git://")
        || lower.starts_with("ssh://")
}

/// Rootless `buildah bud` runner with no ambient environment or network.
pub struct BuildahEngine {
    binary: PathBuf,
    output_prefix: String,
}

impl BuildahEngine {
    /// Creates the runner using an absolute trusted `buildah` executable.
    ///
    /// The output name is internal to an isolated image store; publishing,
    /// scan, signing, and final digest attribution are intentionally supplied
    /// by the platform's engine implementation rather than a tenant command.
    ///
    /// # Errors
    ///
    /// Returns an error unless the executable path and output name are safe.
    pub fn new(binary: PathBuf, output_prefix: String) -> Result<Self, OciWorkerError> {
        if !binary.is_absolute() || output_prefix.trim().is_empty() || output_prefix.len() > 160 {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        Ok(Self {
            binary,
            output_prefix,
        })
    }

    /// Returns the local Buildah tag shared with the output publisher.
    #[must_use]
    pub fn output_name(&self, request: &IsolatedOciBuild) -> String {
        local_image_name(&self.output_prefix, request.image_id)
    }

    /// Returns the auditable isolated invocation. No registry credential or
    /// host socket argument can enter this command.
    #[must_use]
    pub fn command(&self, request: &IsolatedOciBuild) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            // Buildah invokes administrator-installed helpers such as newuidmap.
            // A fixed path preserves isolation from the caller's environment.
            .env("PATH", TRUSTED_SYSTEM_PATH)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("bud")
            .arg("--pull=never")
            .arg("--network=none")
            .arg("--isolation=rootless")
            .arg("--build-context")
            .arg(OsString::from(format!(
                "heph-base=container-image://oci:{}",
                request.base_oci_layout.display()
            )))
            .arg("--file")
            .arg(&request.dockerfile)
            .arg("--tag")
            .arg(self.output_name(request))
            .arg(&request.context);
        command
    }

    /// Runs the isolated rootless build. The OCI output is not eligible for
    /// use until a platform publisher has scanned and attested it.
    ///
    /// # Errors
    ///
    /// Returns the bounded process result without exposing command output,
    /// which can contain tenant-controlled Dockerfile data.
    pub async fn execute(&self, request: &IsolatedOciBuild) -> Result<(), OciWorkerError> {
        if !request.network_disabled || !request.ambient_credentials_disabled {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let status = self
            .command(request)
            .status()
            .await
            .map_err(OciWorkerError::Process)?;
        status
            .success()
            .then_some(())
            .ok_or(OciWorkerError::BuildFailed)
    }
}

/// Builds a deterministic local image tag from an administrator-owned prefix.
#[must_use]
pub fn local_image_name(prefix: &str, image_id: OciImageId) -> String {
    format!("{prefix}-{}", image_id.as_uuid().simple())
}

/// Concrete isolated engine that composes rootless Buildah with the
/// platform-owned scan, provenance, and immutable-publication step.
pub struct PublishedBuildahEngine<P> {
    buildah: BuildahEngine,
    publisher: P,
}

impl<P> PublishedBuildahEngine<P> {
    /// Creates the complete OCI engine.
    #[must_use]
    pub const fn new(buildah: BuildahEngine, publisher: P) -> Self {
        Self { buildah, publisher }
    }
}

#[async_trait]
impl<P> OciBuildEngine for PublishedBuildahEngine<P>
where
    P: OciOutputPublisher,
{
    async fn build(
        &self,
        request: IsolatedOciBuild,
    ) -> Result<OciImageProductionOutput, OciWorkerError> {
        self.buildah.execute(&request).await?;
        self.publisher.publish(&request).await
    }
}

/// Durable worker store failure.
#[derive(Debug, thiserror::Error)]
pub enum OciWorkerStoreError {
    /// Durable storage or transport failed.
    #[error("OCI worker store failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A claim or completion lost its state race.
    #[error("OCI worker job state changed concurrently")]
    Conflict,
}

/// OCI worker failure.
#[derive(Debug, thiserror::Error)]
pub enum OciWorkerError {
    /// Durable job storage failed.
    #[error(transparent)]
    Store(#[from] OciWorkerStoreError),
    /// Worker configuration is unsafe or incomplete.
    #[error("OCI worker configuration is invalid")]
    InvalidConfiguration,
    /// A source path escaped the exact checkout or contained a symlink.
    #[error("OCI image source path is unsafe")]
    UnsafeSourcePath,
    /// A rootfs destination was unsafe or already occupied.
    #[error("OCI rootfs materialization path is unsafe")]
    UnsafeMaterializationPath,
    /// The daemon has not made the verified immutable image available to its
    /// image-cache adapter.
    #[error("immutable OCI image is not available in the daemon cache")]
    ImageNotCached,
    /// Dockerfile syntax did not satisfy the restricted contract.
    #[error("Dockerfile does not satisfy the restricted OCI image contract")]
    InvalidDockerfile,
    /// Dockerfile selected an image other than the approved `heph-base` or a stage.
    #[error("Dockerfile uses an unapproved base image")]
    UnapprovedDockerfileBase,
    /// Dockerfile attempted to fetch a remote ADD or COPY source.
    #[error("Dockerfile uses a remote ADD or COPY source")]
    RemoteDockerfileSource,
    /// Engine output did not supply matching immutable scan/provenance data.
    #[error("OCI image output is invalid")]
    InvalidOutput,
    /// Registry intent, token, publication, or verification failed without
    /// exposing a credential, source path, or remote command output.
    #[error("repository image registry publication failed")]
    RegistryPublication,
    /// Filesystem operation failed.
    #[error("OCI worker filesystem operation failed: {0}")]
    Filesystem(#[source] std::io::Error),
    /// Manifest serialization failed.
    #[error("OCI root manifest serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    /// The isolated OCI command could not be started or observed.
    #[error("isolated OCI image process failed: {0}")]
    Process(#[source] std::io::Error),
    /// The isolated OCI image exited unsuccessfully.
    #[error("isolated OCI image failed")]
    BuildFailed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Clone)]
    struct TestStore {
        job: Arc<Mutex<Option<ClaimedProductionJob>>>,
        materialization_job: Arc<Mutex<Option<ClaimedMaterializationJob>>>,
        completed: Arc<Mutex<Vec<(Uuid, OciImageProductionOutput, RepositoryOciImageProvenance)>>>,
        failed: Arc<Mutex<Vec<(Uuid, String)>>>,
        materialized: Arc<Mutex<Vec<(Uuid, PathBuf)>>>,
        materialization_failed: Arc<Mutex<Vec<(Uuid, String)>>>,
        root_reference: Option<OciImageReference>,
        roots: Arc<Mutex<Vec<MaterializedRoot>>>,
    }

    #[async_trait]
    impl OciImageProductionJobStore for TestStore {
        async fn claim_production(
            &self,
            _worker_name: &str,
            _lease: Duration,
        ) -> Result<Option<ClaimedProductionJob>, OciWorkerStoreError> {
            Ok(self.job.lock().expect("test job lock").take())
        }

        async fn complete_production(
            &self,
            job_id: Uuid,
            _materialization_worker_name: &str,
            output: &OciImageProductionOutput,
            provenance: RepositoryOciImageProvenance,
        ) -> Result<(), OciWorkerStoreError> {
            self.completed.lock().expect("test completed lock").push((
                job_id,
                output.clone(),
                provenance,
            ));
            Ok(())
        }

        async fn fail_production(
            &self,
            job_id: Uuid,
            reason: &str,
        ) -> Result<(), OciWorkerStoreError> {
            self.failed
                .lock()
                .expect("test failed lock")
                .push((job_id, String::from(reason)));
            Ok(())
        }

        async fn claim_materialization(
            &self,
            _worker_name: &str,
            _lease: Duration,
        ) -> Result<Option<ClaimedMaterializationJob>, OciWorkerStoreError> {
            Ok(self
                .materialization_job
                .lock()
                .expect("test materialization job lock")
                .take())
        }

        async fn complete_materialization(
            &self,
            job_id: Uuid,
            root_path: &Path,
        ) -> Result<(), OciWorkerStoreError> {
            self.materialized
                .lock()
                .expect("test materialized lock")
                .push((job_id, root_path.to_path_buf()));
            if let Some(reference) = &self.root_reference {
                self.roots
                    .lock()
                    .expect("test roots lock")
                    .push(MaterializedRoot {
                        image_reference: reference.clone(),
                        root_path: root_path.to_path_buf(),
                    });
            }
            Ok(())
        }

        async fn fail_materialization(
            &self,
            job_id: Uuid,
            reason: &str,
        ) -> Result<(), OciWorkerStoreError> {
            self.materialization_failed
                .lock()
                .expect("test materialization failed lock")
                .push((job_id, String::from(reason)));
            Ok(())
        }

        async fn materialized_roots(
            &self,
            _worker_name: &str,
        ) -> Result<Vec<MaterializedRoot>, OciWorkerStoreError> {
            Ok(self.roots.lock().expect("test roots lock").clone())
        }
    }

    #[derive(Clone)]
    struct TestCheckout {
        source: PreparedSource,
        cleaned: Arc<AtomicBool>,
    }

    #[async_trait]
    impl SourceCheckoutProvider for TestCheckout {
        async fn checkout(
            &self,
            _job: &ClaimedProductionJob,
        ) -> Result<PreparedSource, OciWorkerError> {
            Ok(self.source.clone())
        }

        async fn cleanup(&self, _source: &PreparedSource) -> Result<(), OciWorkerError> {
            self.cleaned.store(true, Ordering::Release);
            Ok(())
        }
    }

    struct TestEngine {
        output: OciImageProductionOutput,
        fail: bool,
    }

    #[async_trait]
    impl OciBuildEngine for TestEngine {
        async fn build(
            &self,
            _request: IsolatedOciBuild,
        ) -> Result<OciImageProductionOutput, OciWorkerError> {
            if self.fail {
                Err(OciWorkerError::BuildFailed)
            } else {
                Ok(self.output.clone())
            }
        }
    }

    #[derive(Clone)]
    struct TestRootfsExporter {
        exported: Arc<AtomicBool>,
    }

    #[async_trait]
    impl OciRootfsExporter for TestRootfsExporter {
        async fn export_rootfs(
            &self,
            _image_reference: &OciImageReference,
            destination: &Path,
        ) -> Result<(), OciWorkerError> {
            fs::write(destination.join("tool"), "fixture root filesystem")
                .map_err(OciWorkerError::Filesystem)?;
            self.exported.store(true, Ordering::Release);
            Ok(())
        }
    }

    fn reference(digest: char) -> OciImageReference {
        OciImageReference::parse(format!(
            "registry.test/image@sha256:{}",
            digest.to_string().repeat(64)
        ))
        .expect("digest-pinned reference")
    }

    fn production_job() -> ClaimedProductionJob {
        ClaimedProductionJob {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            image_id: OciImageId::new(),
            repository_id: Uuid::new_v4(),
            source_revision: "a".repeat(40),
            context_digest: OciDigest::parse(format!("sha256:{}", "b".repeat(64)))
                .expect("context digest"),
            dockerfile_path: RepositoryOciImageSourcePath::parse(String::from("Dockerfile"))
                .expect("Dockerfile path"),
            context_path: RepositoryOciImageSourcePath::parse(String::from("."))
                .expect("context path"),
            base_reference: reference('c'),
        }
    }

    fn source_fixture(root: &Path) -> PreparedSource {
        let source = root.join("source");
        fs::create_dir(&source).expect("source directory");
        fs::write(source.join("Dockerfile"), "FROM heph-base\nCOPY app /app\n")
            .expect("Dockerfile");
        fs::write(source.join("app"), "fixture").expect("source file");
        let base = root.join("base");
        fs::create_dir(&base).expect("base layout directory");
        PreparedSource {
            checkout_root: fs::canonicalize(source).expect("canonical source"),
            base_oci_layout: fs::canonicalize(base).expect("canonical base"),
        }
    }

    fn successful_output(root: &Path) -> OciImageProductionOutput {
        let layout = root.join("layout");
        fs::create_dir(&layout).expect("output layout directory");
        OciImageProductionOutput {
            image_reference: reference('d'),
            image_digest: OciDigest::parse(format!("sha256:{}", "d".repeat(64)))
                .expect("output digest"),
            attestation_reference: String::from("attestation://fixture"),
            sbom_reference: Some(String::from("sbom://fixture")),
            scan_reference: String::from("scan://fixture"),
            local_oci_layout: fs::canonicalize(layout).expect("canonical output layout"),
        }
    }

    #[test]
    fn accepts_only_heph_base_then_named_stages_or_scratch() {
        let dockerfile = "FROM heph-base AS build\nRUN echo ok\nFROM build AS final\nCOPY --from=build /x /x\nFROM scratch\nCOPY --from=final /x /x\n";
        assert!(DockerfilePolicy::validate(dockerfile).is_ok());
    }

    #[test]
    fn rejects_unapproved_or_remote_dockerfile_inputs() {
        assert!(matches!(
            DockerfilePolicy::validate("FROM ubuntu:24.04\n"),
            Err(OciWorkerError::UnapprovedDockerfileBase)
        ));
        assert!(matches!(
            DockerfilePolicy::validate("FROM heph-base\nADD https://example.test/a /a\n"),
            Err(OciWorkerError::RemoteDockerfileSource)
        ));
        assert!(matches!(
            DockerfilePolicy::validate(
                "FROM heph-base\nCOPY [\"https://example.test/a\", \"/a\"]\n"
            ),
            Err(OciWorkerError::RemoteDockerfileSource)
        ));
        assert!(matches!(
            DockerfilePolicy::validate("FROM heph-base \\\n+RUN echo bypass\n"),
            Err(OciWorkerError::InvalidDockerfile)
        ));
    }

    #[test]
    fn buildah_command_has_no_network_pull_or_ambient_environment() {
        let engine = BuildahEngine::new(PathBuf::from("/usr/bin/buildah"), String::from("output"))
            .expect("engine");
        let request = IsolatedOciBuild {
            image_id: OciImageId::new(),
            project_id: Uuid::new_v4(),
            dockerfile: PathBuf::from("/source/Dockerfile"),
            context: PathBuf::from("/source"),
            base_oci_layout: PathBuf::from("/bases/ubuntu"),
            base_reference: OciImageReference::parse(format!(
                "registry.test/base@sha256:{}",
                "a".repeat(64)
            ))
            .expect("reference"),
            network_disabled: true,
            ambient_credentials_disabled: true,
        };
        let command = engine.command(&request);
        let debug = format!("{command:?}");
        assert!(debug.contains("--network=none"));
        assert!(debug.contains("--pull=never"));
        assert!(debug.contains("heph-base=container-image://oci:/bases/ubuntu"));
        assert!(debug.contains(TRUSTED_SYSTEM_PATH));
        assert!(!debug.contains("token"));
        assert!(!debug.contains("authfile"));
    }

    #[tokio::test]
    async fn durable_production_records_verified_output_and_cleans_the_exact_checkout() {
        let temporary = tempfile::tempdir().expect("temporary source tree");
        let job = production_job();
        let store = TestStore {
            job: Arc::new(Mutex::new(Some(job.clone()))),
            materialization_job: Arc::new(Mutex::new(None)),
            completed: Arc::new(Mutex::new(Vec::new())),
            failed: Arc::new(Mutex::new(Vec::new())),
            materialized: Arc::new(Mutex::new(Vec::new())),
            materialization_failed: Arc::new(Mutex::new(Vec::new())),
            root_reference: None,
            roots: Arc::new(Mutex::new(Vec::new())),
        };
        let cleaned = Arc::new(AtomicBool::new(false));
        let worker = OciImageProductionWorker::new(
            store.clone(),
            TestCheckout {
                source: source_fixture(temporary.path()),
                cleaned: Arc::clone(&cleaned),
            },
            TestEngine {
                output: successful_output(temporary.path()),
                fail: false,
            },
            String::from("production-worker"),
            String::from("rootfs-worker"),
            Duration::from_secs(30),
        )
        .expect("worker configuration");

        assert!(worker.run_once().await.expect("production pass"));
        assert!(cleaned.load(Ordering::Acquire));
        {
            let completed = store.completed.lock().expect("test completed lock");
            assert_eq!(completed.len(), 1);
            assert_eq!(completed[0].0, job.id);
            assert_eq!(completed[0].2.source_revision, job.source_revision);
            assert_eq!(completed[0].2.context_digest, job.context_digest);
            drop(completed);
        }
        assert!(store.failed.lock().expect("test failed lock").is_empty());
    }

    #[tokio::test]
    async fn failed_publication_never_completes_or_makes_a_image_ready() {
        let temporary = tempfile::tempdir().expect("temporary source tree");
        let job = production_job();
        let store = TestStore {
            job: Arc::new(Mutex::new(Some(job.clone()))),
            materialization_job: Arc::new(Mutex::new(None)),
            completed: Arc::new(Mutex::new(Vec::new())),
            failed: Arc::new(Mutex::new(Vec::new())),
            materialized: Arc::new(Mutex::new(Vec::new())),
            materialization_failed: Arc::new(Mutex::new(Vec::new())),
            root_reference: None,
            roots: Arc::new(Mutex::new(Vec::new())),
        };
        let cleaned = Arc::new(AtomicBool::new(false));
        let worker = OciImageProductionWorker::new(
            store.clone(),
            TestCheckout {
                source: source_fixture(temporary.path()),
                cleaned: Arc::clone(&cleaned),
            },
            TestEngine {
                output: successful_output(temporary.path()),
                fail: true,
            },
            String::from("production-worker"),
            String::from("rootfs-worker"),
            Duration::from_secs(30),
        )
        .expect("worker configuration");

        assert!(worker.run_once().await.expect("production pass"));
        assert!(cleaned.load(Ordering::Acquire));
        {
            let failed = store.failed.lock().expect("test failed lock");
            assert_eq!(failed.len(), 1);
            assert_eq!(failed[0].0, job.id);
            assert_eq!(failed[0].1, "isolated OCI image failed");
            drop(failed);
        }
        assert!(
            store
                .completed
                .lock()
                .expect("test completed lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn materialization_exports_an_immutable_root_and_writes_the_digest_manifest() {
        let temporary = tempfile::tempdir().expect("temporary root tree");
        let output = successful_output(temporary.path());
        let materialization = ClaimedMaterializationJob {
            id: Uuid::new_v4(),
            image_reference: output.image_reference.clone(),
        };
        let store = TestStore {
            job: Arc::new(Mutex::new(None)),
            materialization_job: Arc::new(Mutex::new(Some(materialization.clone()))),
            completed: Arc::new(Mutex::new(Vec::new())),
            failed: Arc::new(Mutex::new(Vec::new())),
            materialized: Arc::new(Mutex::new(Vec::new())),
            materialization_failed: Arc::new(Mutex::new(Vec::new())),
            root_reference: Some(materialization.image_reference.clone()),
            roots: Arc::new(Mutex::new(Vec::new())),
        };
        let exported = Arc::new(AtomicBool::new(false));
        let rootfs_root = temporary.path().join("rootfs");
        let worker = RootfsMaterializationWorker::new(
            store.clone(),
            TestRootfsExporter {
                exported: Arc::clone(&exported),
            },
            String::from("rootfs-worker"),
            rootfs_root,
            Duration::from_secs(30),
        )
        .expect("materialization worker configuration");

        assert!(worker.run_once().await.expect("materialization pass"));
        assert!(exported.load(Ordering::Acquire));
        {
            let completed = store.materialized.lock().expect("test materialized lock");
            assert_eq!(completed.len(), 1);
            assert_eq!(completed[0].0, materialization.id);
            assert!(completed[0].1.join("tool").is_file());
            drop(completed);
        }
        let manifest = temporary.path().join("image-roots.json");
        worker
            .write_manifest(&manifest)
            .await
            .expect("root manifest");
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(manifest).expect("manifest bytes"))
                .expect("manifest JSON");
        assert_eq!(document["version"], 1);
        assert_eq!(
            document["roots"][materialization.image_reference.as_str()]["kind"],
            "directory"
        );
        assert!(
            store
                .materialization_failed
                .lock()
                .expect("test materialization failed lock")
                .is_empty()
        );
    }
}
