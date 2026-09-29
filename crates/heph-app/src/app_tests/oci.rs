use crate::oci_workers::refresh_image_filesystem_cache;
use crate::{
    BuildExecutionError, MaterializedRoot, OciImageReference, OciWorkerError,
    build_delivery_requires_redelivery, write_oci_manifest_if_dirty,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn claimed_build_delivery_is_acknowledged_instead_of_poison_redelivered() {
    assert!(!build_delivery_requires_redelivery(
        &BuildExecutionError::AlreadyClaimed
    ));
    assert!(!build_delivery_requires_redelivery(
        &BuildExecutionError::GuestFailed
    ));
    assert!(build_delivery_requires_redelivery(
        &BuildExecutionError::Release
    ));
    assert!(build_delivery_requires_redelivery(
        &BuildExecutionError::ImageUnavailable
    ));
}

#[tokio::test]
async fn failed_manifest_write_is_retried_when_next_pass_has_no_job() {
    let dirty = AtomicBool::new(false);
    let first = write_oci_manifest_if_dirty(&dirty, true, || async {
        Err(OciWorkerError::ImageNotCached)
    })
    .await;
    assert!(first.is_err());
    assert!(dirty.load(Ordering::Acquire));

    let attempts = AtomicUsize::new(0);
    write_oci_manifest_if_dirty(&dirty, false, || async {
        attempts.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })
    .await
    .expect("dirty manifest is retried without another materialization job");
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
    assert!(!dirty.load(Ordering::Acquire));
}

#[test]
fn durable_materialized_root_hydrates_image_cache() {
    let rootfs = tempfile::tempdir().expect("temporary rootfs");
    let root = rootfs.path().join("materialized");
    std::fs::create_dir(&root).expect("materialized root");
    let reference =
        OciImageReference::parse(format!("localhost/python-ubuntu@sha256:{}", "a".repeat(64)))
            .expect("digest-pinned image reference");
    let image_filesystems =
        std::sync::Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new()));

    refresh_image_filesystem_cache(
        &[MaterializedRoot {
            image_reference: reference.clone(),
            root_path: root.clone(),
        }],
        rootfs.path(),
        &image_filesystems,
    )
    .expect("durable root refresh");

    let cache = image_filesystems.read().expect("image cache read");
    let Some(heph_runtime::RootFilesystem::Directory { host_path }) =
        cache.get(&reference.to_string())
    else {
        panic!("durable root was not hydrated into image cache");
    };
    assert_eq!(
        host_path,
        &std::fs::canonicalize(root).expect("canonical root")
    );
    drop(cache);
}
