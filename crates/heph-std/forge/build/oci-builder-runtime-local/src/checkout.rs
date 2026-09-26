//! Source checkout implementation for the local runtime.

use super::{
    config::LocalOciRuntime,
    constants::TRUSTED_SYSTEM_PATH,
    output::{canonical_directory, prepare_job_checkout},
};
use async_trait::async_trait;
use oci_builder_worker::{
    ClaimedProductionJob, OciWorkerError, PreparedSource, SourceCheckoutProvider,
};
use std::{fs, process::Stdio};
use tokio::process::Command;

#[async_trait]
impl SourceCheckoutProvider for LocalOciRuntime {
    async fn checkout(&self, job: &ClaimedProductionJob) -> Result<PreparedSource, OciWorkerError> {
        let repository = self
            .config
            .repository_root
            .join(format!("{}.git", job.repository_id));
        let repository = canonical_directory(&repository)?;
        if repository.parent() != Some(self.config.repository_root.as_path()) {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        // A worker may have died after materializing this durable job's source
        // but before its cleanup ran. Reclaim only the exact, direct child
        // owned by that job; never follow an unexpected link or remove an
        // arbitrary path left beneath the private checkout root.
        let checkout = prepare_job_checkout(&self.config.checkout_root, job.id)?;
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
            .image_layouts
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
