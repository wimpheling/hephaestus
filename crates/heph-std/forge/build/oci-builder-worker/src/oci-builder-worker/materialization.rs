use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

use crate::policy::{bounded_reason, digest_directory_name};
use crate::{
    ClaimedMaterializationJob, MaterializedRoot, OciImageProductionJobStore, OciRootfsExporter,
    OciWorkerError,
};

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

pub fn install_guest_init(root: &Path, guest_init: &Path) -> Result<(), OciWorkerError> {
    let mut directory = root.to_path_buf();
    for component in ["usr", "libexec", "hephaestus"] {
        directory.push(component);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => return Err(OciWorkerError::UnsafeMaterializationPath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&directory).map_err(OciWorkerError::Filesystem)?;
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))
                    .map_err(OciWorkerError::Filesystem)?;
            }
            Err(error) => return Err(OciWorkerError::Filesystem(error)),
        }
    }
    let destination = directory.join("heph-init");
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => return Err(OciWorkerError::UnsafeMaterializationPath),
        Err(error) => return Err(OciWorkerError::Filesystem(error)),
    }
    fs::copy(guest_init, &destination).map_err(OciWorkerError::Filesystem)?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
        .map_err(OciWorkerError::Filesystem)
}

/// Daemon-local rootfs materialization worker.
pub struct RootfsMaterializationWorker<S, E> {
    store: S,
    exporter: E,
    worker_name: String,
    rootfs_root: PathBuf,
    guest_init: Option<PathBuf>,
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
            guest_init: None,
            lease,
        })
    }

    /// Installs the reviewed guest bootstrap into each newly materialized
    /// execution root. Repository Dockerfiles cannot provide or replace it.
    ///
    /// # Errors
    ///
    /// Returns an error unless the configured bootstrap is an absolute regular
    /// host file owned by the daemon configuration.
    pub fn with_guest_init(mut self, guest_init: PathBuf) -> Result<Self, OciWorkerError> {
        if !guest_init.is_absolute() {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let metadata = fs::symlink_metadata(&guest_init).map_err(OciWorkerError::Filesystem)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        self.guest_init = Some(fs::canonicalize(guest_init).map_err(OciWorkerError::Filesystem)?);
        Ok(self)
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
        if let Some(guest_init) = &self.guest_init
            && let Err(error) = install_guest_init(staging, guest_init)
        {
            let _ = fs::remove_dir_all(staging);
            return Err(error);
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

    /// Lists only durable, successfully materialized roots owned by this
    /// daemon worker. Callers must still validate the returned host paths
    /// before supplying them to a VM provider.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable materialization store is unavailable.
    pub async fn materialized_roots(&self) -> Result<Vec<MaterializedRoot>, OciWorkerError> {
        self.store
            .materialized_roots(&self.worker_name)
            .await
            .map_err(OciWorkerError::Store)
    }
}
