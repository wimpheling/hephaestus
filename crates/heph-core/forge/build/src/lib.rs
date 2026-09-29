//! Provider-neutral contracts shared by OCI build adapters and workers.

use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageId, OciImageReference};
use registry_domain::{OciDescriptor, PublicationIntent, PublicationIntentId, VerifiedPublication};
use serde::Serialize;
use std::{path::Path, path::PathBuf, time::Duration};
use uuid::Uuid;

/// A validated repository-relative Dockerfile or OCI build-context path.
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
    /// The daemon has not made the verified immutable image available in the daemon cache.
    #[error("immutable OCI image is not available in the daemon cache")]
    ImageNotCached,
    /// Dockerfile syntax did not satisfy the restricted OCI image contract.
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
    /// Registry intent, token, publication, or verification failed.
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
    /// A dedicated builder or verifier VM did not complete successfully.
    #[error("isolated OCI {phase} VM failed (safe exit code {exit_code:?})")]
    IsolatedVmFailed {
        /// Fixed platform operation phase.
        phase: &'static str,
        /// Guest process exit code when the VM reported one.
        exit_code: Option<i32>,
    },
}

#[cfg(test)]
mod tests {
    use super::{OciWorkerError, RepositoryOciImageProvenance, RepositoryOciImageSourcePath};
    use builder_catalog_domain::OciDigest;

    #[test]
    fn source_paths_reject_traversal_and_accept_repository_paths() {
        assert!(RepositoryOciImageSourcePath::parse("Dockerfile").is_ok());
        assert!(RepositoryOciImageSourcePath::parse("src/Dockerfile").is_ok());
        assert!(matches!(
            RepositoryOciImageSourcePath::parse("../Dockerfile"),
            Err(OciWorkerError::UnsafeSourcePath)
        ));
        assert!(matches!(
            RepositoryOciImageSourcePath::parse("/tmp/Dockerfile"),
            Err(OciWorkerError::UnsafeSourcePath)
        ));
    }

    #[test]
    fn provenance_requires_lowercase_immutable_revision_and_references() {
        let provenance = RepositoryOciImageProvenance {
            source_revision: "a".repeat(40),
            context_digest: OciDigest::parse(format!("sha256:{}", "b".repeat(64)))
                .expect("valid digest"),
            attestation_reference: String::from("attestation:v1"),
            sbom_reference: Some(String::from("sbom:v1")),
        };
        assert!(provenance.validate().is_ok());

        let invalid = RepositoryOciImageProvenance {
            source_revision: "A".repeat(40),
            ..provenance
        };
        assert!(matches!(
            invalid.validate(),
            Err(OciWorkerError::InvalidOutput)
        ));
    }
}
