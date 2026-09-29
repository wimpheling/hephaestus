use builder_catalog_domain::{OciImageId, OciImageReference};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

use crate::buildah::TRUSTED_SYSTEM_PATH;
use crate::*;

#[path = "test_support.rs"]
mod support;
use support::*;

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
        DockerfilePolicy::validate("FROM heph-base\nCOPY [\"https://example.test/a\", \"/a\"]\n"),
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
        job_id: Uuid::new_v4(),
        image_id: OciImageId::new(),
        project_id: Uuid::new_v4(),
        dockerfile: PathBuf::from("/source/Dockerfile"),
        checkout_root: PathBuf::from("/source"),
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

#[test]
fn guest_init_installation_rejects_a_repository_supplied_target() {
    let temporary = tempfile::tempdir().expect("temporary root tree");
    let bootstrap = temporary.path().join("heph-init");
    fs::write(&bootstrap, b"reviewed bootstrap").expect("bootstrap");
    let root = temporary.path().join("rootfs");
    fs::create_dir_all(root.join("usr/libexec/hephaestus")).expect("rootfs");

    install_guest_init(&root, &bootstrap).expect("first installation");
    assert_eq!(
        fs::read(root.join("usr/libexec/hephaestus/heph-init")).expect("installed bootstrap"),
        b"reviewed bootstrap"
    );
    assert!(matches!(
        install_guest_init(&root, &bootstrap),
        Err(OciWorkerError::UnsafeMaterializationPath)
    ));
}
