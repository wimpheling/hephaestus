//! Existing unchanged scenario bodies; parent retains original test names.
use super::super::support::*;
use super::spawner;
use crate::provider::{
    launch::{LaunchState, PendingLaunch},
    ownership::ProviderOwner,
};

pub(super) async fn raw_child_survives_placement_failure_without_transport_and_is_actually_reaped()
{
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    let worker = temp.path().join("nonconnecting-worker");
    fs::write(&worker, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
    config.worker_binary = worker;
    let requested = spec("raw-placement", root);
    let prepared = prepare_spec(&config, &requested).unwrap();
    fs::create_dir(cgroups.join(&requested.id.0)).unwrap();
    // A directory cannot accept the placement control write.
    fs::create_dir(cgroups.join(&requested.id.0).join("cgroup.procs")).unwrap();
    let (entry, _retries) = PendingLaunch::new(requested.id.clone(), &config, None).unwrap();
    drop(
        crate::directory::ChildDirectory::create(
            &runtime,
            &requested.id.0,
            rustix::fs::Mode::from_bits_truncate(0o700),
            |allocation| entry.custody.retain_runtime(allocation).unwrap(),
        )
        .unwrap(),
    );
    let error = crate::provider::common::WorkerClient::launch(
        Arc::new(config.clone()),
        prepared,
        &runtime.join(&requested.id.0),
        &Cgroup::existing(&config, &requested.id.0),
        &entry.custody,
    )
    .await
    .err()
    .unwrap();
    assert!(error.to_string().contains("cgroup-place-worker"));
    let process = entry.custody.process().unwrap().unwrap();
    let exit = process.subscribe_process_exit();
    assert!(process.pid.is_some());
    entry
        .custody
        .terminate(Duration::from_secs(1))
        .await
        .unwrap();
    assert!(
        exit.borrow().is_some(),
        "actual Child reap must publish a real exit"
    );
    // This checks local Child kill/wait only. The ordinary filesystem cannot
    // establish real cgroup membership, descendant drain, or VM containment.
    entry
        .custody
        .terminate(Duration::from_secs(1))
        .await
        .unwrap();
    drop(exit);
    drop(process);
    drop(entry);
}

pub(super) async fn exclusive_mkdir_failure_never_adopts_or_deletes_foreign_children() {
    for foreign_runtime in [false, true] {
        let temp = TempDir::new().unwrap();
        let (config, root, runtime, cgroups) = emulated_config(&temp);
        let spawner = spawner(false);
        let provider = LibkrunProvider::new_with_spawner(config.clone(), spawner.clone()).unwrap();
        let owner = ProviderOwner::initialize(&config, "test-host").unwrap();
        assert!(provider.ownership_test_inner().owner.set(owner).is_ok());
        let requested = spec("foreign-child", root);
        let parent = if foreign_runtime { &runtime } else { &cgroups };
        let foreign = parent.join(&requested.id.0);
        fs::create_dir(&foreign).unwrap();
        let sentinel = foreign.join("sentinel");
        fs::write(&sentinel, b"foreign bytes").unwrap();
        assert!(provider.provision(requested.clone()).await.is_err());
        assert_eq!(fs::read(sentinel).unwrap(), b"foreign bytes");
        assert!(
            !provider
                .ownership_test_inner()
                .ids
                .lock()
                .await
                .contains(&requested.id)
        );
        assert!(
            provider
                .ownership_test_inner()
                .pending_launches
                .lock()
                .unwrap()
                .is_empty()
        );
        assert_eq!(spawner.worker.reaps.load(Ordering::Relaxed), 0);
        if !foreign_runtime {
            assert!(!runtime.join(&requested.id.0).exists());
        }
        drop(provider);
        drop(spawner);
    }
}

pub(super) async fn cancelled_nonconnecting_raw_launch_is_reaped_before_id_and_files_release() {
    let temp = TempDir::new().unwrap();
    let (mut config, root, runtime, cgroups) = emulated_config(&temp);
    let binary = temp.path().join("nonconnecting-worker");
    fs::write(&binary, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    config.worker_binary = binary;
    let provider = LibkrunProvider::new(config).unwrap();
    let requested = spec("cancel-raw-ipc", root);
    let id = requested.id.clone();
    let task = tokio::spawn({
        let provider = provider.clone();
        async move { provider.provision(requested).await }
    });
    let (entry, process) = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            {
                let entry = provider
                    .ownership_test_inner()
                    .pending_launches
                    .lock()
                    .unwrap()
                    .get(&id)
                    .cloned();
                if let Some(entry) = entry {
                    if let Some(process) = entry.custody.process().unwrap() {
                        break (entry, process);
                    }
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let exit = process.subscribe_process_exit();
    assert!(exit.borrow().is_none());
    assert!(
        provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let mut state = entry.state.subscribe();
    tokio::time::timeout(
        Duration::from_secs(1),
        state.wait_for(|state| matches!(state, LaunchState::Cleaned)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        exit.borrow().is_some(),
        "actual raw Child reap precedes cleanup completion"
    );
    assert!(
        !provider
            .ownership_test_inner()
            .ids
            .lock()
            .await
            .contains(&id)
    );
    assert!(!runtime.join(&id.0).exists());
    assert!(!cgroups.join(&id.0).exists());
    // Emulated cgroup files cannot prove native containment or descendant drain.
    drop(state);
    drop(exit);
    drop(process);
    drop(entry);
    drop(provider);
}
