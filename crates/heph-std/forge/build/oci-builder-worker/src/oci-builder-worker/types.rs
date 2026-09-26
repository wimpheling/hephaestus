use async_trait::async_trait;
use builder_catalog_domain::{OciImageId, OciImageReference};
use registry_domain::PublicationIntent;
use registry_token::IssuedToken;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::OciWorkerError;
use heph_build::{ClaimedProductionJob, OciImageProductionOutput};

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
    /// Durable preparation attempt that owns every transient VM workspace.
    pub job_id: Uuid,
    /// Repository OCI image definition being prepared.
    pub image_id: OciImageId,
    /// Opaque durable project identity that owns this image.
    pub project_id: Uuid,
    /// Absolute canonical Dockerfile path.
    pub dockerfile: PathBuf,
    /// Absolute canonical root of the exact read-only checkout.
    pub checkout_root: PathBuf,
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
