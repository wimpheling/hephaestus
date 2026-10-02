use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
};
use vm_trait::{VmError, VmProvider};

use crate::{INTEGRATION_TEST_LOCK, support::collect_logs_until_any_exit};

#[path = "named_volumes/fixture.rs"]
mod fixture;

#[tokio::test(flavor = "multi_thread")]
async fn kernel_enforces_named_read_only_and_writable_volume_contracts() {
    if std::env::var("HEPHAESTUS_LIBKRUN_NAMED_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let _test_lock = INTEGRATION_TEST_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    assert_ne!(rustix::process::geteuid().as_raw(), 0);
    let fixture = fixture::Fixture::new();
    let before = fixture::hash(&fixture.read_only);
    let spec = fixture.spec("named-positive");
    let vm = fixture
        .provider
        .provision(spec)
        .await
        .expect("provision named VM");
    let mut events = vm.subscribe_events();
    if let Err(error) = vm.start().await {
        vm.destroy()
            .await
            .expect("destroy failed positive-start VM");
        panic!("start fresh protocol10 guest: {error:?}");
    }
    let (logs, exit) = collect_logs_until_any_exit(&mut events).await;
    vm.destroy()
        .await
        .expect("confirmed positive VM destruction");
    fixture.assert_clean("named-positive");
    assert_eq!(
        fixture::hash(&fixture.read_only),
        before,
        "RO backing bytes changed"
    );
    assert_eq!(exit.code, Some(0), "guest output:\n{logs}");
    for marker in [
        "NAMED_VOLUME_GUEST=1",
        "uid=0",
        "paths_uuid=1",
        "sysfs_ro=1",
        "blkroget=1",
        "ro_file_denied=1",
        "ro_raw_denied=1",
        "ro_remount_denied=1",
        "rw_remount_control=1",
        "rw_raw_control=1",
        "rw_ioctl_control=1",
        "ro_backend_durable_denied=1",
        "rw_write=1",
        "no_sqlite_init=1",
    ] {
        assert!(logs.contains(marker), "missing {marker}: {logs}");
    }
    println!("{logs}");
    assert_eq!(
        fixture::read_persisted(&fixture.writable),
        "named-writable-persisted\n"
    );

    rejects_dirty_read_only_before_workload(&fixture).await;
    println!(
        "REAL_NAMED_VOLUME_KVM=1 protocol=10 privileged_guest_controls=1 ro_file_raw_remount_denied=1 kernel_device_ro=1 ro_backing_hash_unchanged=1 ro_sha256={before} rw_data_persisted=1 dirty_ro_rejected=1 vm_destroyed=1"
    );
}

async fn rejects_dirty_read_only_before_workload(fixture: &fixture::Fixture) {
    let dirty = fixture.directory.path().join("dirty.raw");
    std::fs::copy(&fixture.read_only, &dirty).expect("copy task-owned RO fixture");
    let mut image = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&dirty)
        .expect("owned dirty image");
    image.seek(SeekFrom::Start(1024 + 58)).unwrap();
    let mut original_state = [0_u8; 2];
    image.read_exact(&mut original_state).unwrap();
    let original_state = u16::from_le_bytes(original_state);
    assert_eq!(original_state & 1, 1);
    image.seek(SeekFrom::Start(1024 + 58)).unwrap();
    image.write_all(&[0, 0]).unwrap();
    image.sync_all().unwrap();
    drop(image);
    let dirty_before = fixture::hash(&dirty);
    let mut spec = fixture.spec("named-dirty");
    spec.disks[0].host_path.clone_from(&dirty);
    spec.command
        .env
        .insert("HEPH_TEST_DIRTY_MARKER".to_owned(), "1".to_owned());
    let vm = fixture
        .provider
        .provision(spec)
        .await
        .expect("provision dirty RO test");
    let failure = vm
        .start()
        .await
        .expect_err("dirty RO must not admit workload");
    vm.destroy()
        .await
        .expect("confirmed failed-start VM destruction");
    fixture.assert_clean("named-dirty");
    assert_eq!(
        fixture::hash(&dirty),
        dirty_before,
        "dirty RO bytes changed"
    );
    assert!(
        matches!(failure, VmError::Unavailable { ref resource, ref reason }
        if resource == "guest readiness" && reason.starts_with("guest exited before readiness")),
        "unexpected failure: {failure:?}"
    );
    assert!(fixture::read_data_file(&fixture.writable, "cat /dirty-workload-executed").is_empty());
    println!(
        "NAMED_VOLUME_DIRTY=1 ext4_state_before={original_state} ext4_state_after=0 workload_not_started=1"
    );
}
