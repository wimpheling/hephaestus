use async_trait::async_trait;
use builder_catalog_domain::OciImageId;
use std::{ffi::OsString, path::PathBuf, process::Stdio};
use tokio::process::Command;

use crate::{
    IsolatedOciBuild, OciBuildEngine, OciImageProductionOutput, OciOutputPublisher, OciWorkerError,
};

pub const TRUSTED_SYSTEM_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

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
