use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageId, OciImageReference};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

use crate::*;

#[derive(Clone)]
pub(super) struct TestStore {
    pub(super) job: Arc<Mutex<Option<ClaimedProductionJob>>>,
    pub(super) materialization_job: Arc<Mutex<Option<ClaimedMaterializationJob>>>,
    pub(super) completed:
        Arc<Mutex<Vec<(Uuid, OciImageProductionOutput, RepositoryOciImageProvenance)>>>,
    pub(super) failed: Arc<Mutex<Vec<(Uuid, String)>>>,
    pub(super) materialized: Arc<Mutex<Vec<(Uuid, PathBuf)>>>,
    pub(super) materialization_failed: Arc<Mutex<Vec<(Uuid, String)>>>,
    pub(super) root_reference: Option<OciImageReference>,
    pub(super) roots: Arc<Mutex<Vec<MaterializedRoot>>>,
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

    async fn fail_production(&self, job_id: Uuid, reason: &str) -> Result<(), OciWorkerStoreError> {
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
pub(super) struct TestCheckout {
    pub(super) source: PreparedSource,
    pub(super) cleaned: Arc<AtomicBool>,
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

pub(super) struct TestEngine {
    pub(super) output: OciImageProductionOutput,
    pub(super) fail: bool,
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
pub(super) struct TestRootfsExporter {
    pub(super) exported: Arc<AtomicBool>,
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

pub(super) fn reference(digest: char) -> OciImageReference {
    OciImageReference::parse(format!(
        "registry.test/image@sha256:{}",
        digest.to_string().repeat(64)
    ))
    .expect("digest-pinned reference")
}

pub(super) fn production_job() -> ClaimedProductionJob {
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
        context_path: RepositoryOciImageSourcePath::parse(String::from(".")).expect("context path"),
        base_reference: reference('c'),
    }
}

pub(super) fn source_fixture(root: &Path) -> PreparedSource {
    let source = root.join("source");
    fs::create_dir(&source).expect("source directory");
    fs::write(source.join("Dockerfile"), "FROM heph-base\nCOPY app /app\n").expect("Dockerfile");
    fs::write(source.join("app"), "fixture").expect("source file");
    let base = root.join("base");
    fs::create_dir(&base).expect("base layout directory");
    PreparedSource {
        checkout_root: fs::canonicalize(source).expect("canonical source"),
        base_oci_layout: fs::canonicalize(base).expect("canonical base"),
    }
}

pub(super) fn successful_output(root: &Path) -> OciImageProductionOutput {
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
