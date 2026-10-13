//! Fresh-init protocol11 ordinary-user VM proof, separate from UID0 controls.

#[path = "guest_initialization/fixture.rs"]
mod fixture;
#[path = "guest_initialization/images.rs"]
mod images;

use std::{path::Path, time::Duration};
use vm_trait::VmProvider;

use crate::{INTEGRATION_TEST_LOCK, support::collect_logs_until_any_exit};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires fresh musl initializer/probe, KVM, ext4 tools and delegated cgroups"]
async fn ordinary_user_typed_builtin_state_and_named_none_survive_restart() {
    let _test_lock = INTEGRATION_TEST_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    assert_ne!(
        rustix::process::geteuid().as_raw(),
        0,
        "native host provider must run unprivileged"
    );
    assert_eq!(vm_libkrun::protocol::PROTOCOL_VERSION, 11);
    let mut fixture = fixture::Fixture::new();
    let ro_before = images::hash(&fixture.facts.path);
    let seeded_before = fixture.seeded.database_bytes("state.db");
    for mode in ["fresh", "reopen", "seeded"] {
        execute(&fixture, mode).await;
        assert_eq!(
            images::hash(&fixture.facts.path),
            ro_before,
            "read-only image changed in {mode}"
        );
        for image in [
            &fixture.state,
            &fixture.seeded,
            &fixture.data,
            &fixture.facts,
        ] {
            image.assert_uuid();
        }
    }
    assert_eq!(
        fixture.seeded.database_bytes("state.db"),
        seeded_before,
        "existing schema/data/journal bytes rewritten"
    );
    fixture.state.assert_database(
        "state.db",
        "SELECT value FROM application_owned",
        "state persisted",
    );
    fixture.seeded.assert_database(
        "state.db",
        "SELECT value FROM preexisting_schema",
        "seed preserved",
    );
    fixture.data.assert_database(
        "workload.db",
        "SELECT value FROM workload_owned",
        "data persisted",
    );
    fixture.mark_verified();
    drop(fixture);
    println!(
        "REAL_GUEST_INITIALIZATION_KVM=1 protocol=11 uid=10001 gid=10001 builtin_wal_created=1 existing_schema_data_journal_bytes_preserved=1 builtin_restart=1 named_none_no_builtin_db=1 app_sqlite_write_reopen=1 mount_uuid_preserved=1 ordinary_ro_denied=1 ro_backing_hash_unchanged=1 ro_sha256={ro_before} vm_destroyed=1"
    );
}

async fn execute(fixture: &fixture::Fixture, mode: &str) {
    println!(
        "GUEST_INITIALIZATION_WORKER=1 executable_sha256={}",
        images::hash(Path::new(env!(
            "CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"
        )))
    );
    let vm = tokio::time::timeout(
        Duration::from_secs(40),
        fixture.provider.provision(fixture.spec(mode)),
    )
    .await
    .expect("bounded native provisioning")
    .expect("native provision");
    let pids = fixture.worker_pids(mode);
    let mut events = vm.subscribe_events();
    let start = tokio::time::timeout(Duration::from_secs(40), vm.start()).await;
    if !matches!(start, Ok(Ok(()))) {
        tokio::time::timeout(Duration::from_secs(30), vm.destroy())
            .await
            .expect("failed-start destruction timeout")
            .expect("failed-start destruction");
        fixture.assert_clean(mode, &pids);
        panic!("protocol11 guest start failed: {start:?}");
    }
    let execution = tokio::time::timeout(
        Duration::from_secs(60),
        collect_logs_until_any_exit(&mut events),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(30), vm.destroy())
        .await
        .expect("destruction timeout")
        .expect("confirmed native VM destruction");
    drop(vm);
    fixture.assert_clean(mode, &pids);
    let (logs, exit) = execution.expect("bounded native workload");
    assert_eq!(exit.code, Some(0), "guest output:\n{logs}");
    for marker in [
        "GUEST_INITIALIZATION=1",
        "uid=10001",
        "gid=10001",
        "device_identity=1",
        "kernel_ro=1",
        "ro_file_denied=1",
        "none_no_builtin_db=1",
        "sqlite_integrity=1",
        "BUILTIN_STATE=1",
        "schema_data_preserved=1",
        "NAMED_NONE=1",
        "app_owned_sqlite=1",
        "write_reopen=1",
    ] {
        assert!(logs.contains(marker), "missing {marker}: {logs}");
    }
    println!("{logs}");
}
