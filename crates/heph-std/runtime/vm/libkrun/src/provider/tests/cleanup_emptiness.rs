//! Controlled resource retention and known-absence retry, not native drain.
use super::support::*;

#[tokio::test]
async fn uncertain_cgroup_retains_runtime_resources_and_id_after_worker_reap() {
    let temp = TempDir::new().unwrap();
    let worker = Arc::new(MockWorker::new());
    let vm = instance(&temp, worker.clone());
    let cgroup = Cgroup::existing(&vm.config, &vm.id.0);
    fs::create_dir(cgroup.path()).unwrap();
    let runtime = temp.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    let sentinel = runtime.join("sentinel");
    fs::write(&sentinel, b"owned bytes").unwrap();
    *vm.resources.lock().await = Some(OwnedResources {
        runtime_dir: runtime.clone(),
        runtime_child: None,
        birth_record: None,
        cgroup,
        cleanup_roots: None,
        service_broker: None,
    });
    let events = vm.config.cgroup_root.join(&vm.id.0).join("cgroup.events");
    for observation in [None, Some("populated invalid\n")] {
        if let Some(contents) = observation {
            fs::write(&events, contents).unwrap();
        }
        assert!(
            vm.destroy()
                .await
                .unwrap_err()
                .to_string()
                .contains("cgroup-events")
        );
        assert!(
            worker.process_exit.borrow().is_some(),
            "controlled worker already reaped"
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"owned bytes");
        assert!(vm.resources.lock().await.is_some());
        assert!(vm.provider_ids.ids.lock().await.contains(vm.id()));
        assert!(!vm.cleanup_complete.load(Ordering::Acquire));
        assert!(matches!(vm.start().await, Err(VmError::Destroyed)));
    }
    fs::write(&events, "populated 0\n").unwrap();
    assert!(
        vm.destroy().await.is_err(),
        "regular fixture control file prevents physical rmdir"
    );
    assert!(runtime.is_dir());
    assert!(vm.provider_ids.ids.lock().await.contains(vm.id()));
    fs::remove_file(&events).unwrap();
    fs::remove_dir(vm.config.cgroup_root.join(&vm.id.0)).unwrap();
    // External completed removal is a controlled exact-child absence witness;
    // actual virtual control files and populated=0 require C's native gate.
    vm.destroy().await.unwrap();
    vm.destroy().await.unwrap();
    assert!(!runtime.exists());
    assert!(vm.resources.lock().await.is_none());
    assert!(!vm.provider_ids.ids.lock().await.contains(vm.id()));
    assert!(vm.cleanup_complete.load(Ordering::Acquire));
    drop(vm);
}
