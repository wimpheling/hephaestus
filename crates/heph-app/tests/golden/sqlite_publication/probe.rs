use crate::*;
use heph_runtime::{GuestCommand, NetworkMode, VmEvent, VmId, VmProvider, VmResources, VmSpec};

pub async fn verify(config: &AppConfig, root: &Path) {
    let VmBackendConfig::Libkrun(provider_config) = &config.vm_backend else {
        panic!("actual libkrun required")
    };
    let provider = vm_libkrun::LibkrunProvider::new((**provider_config).clone())
        .expect("real Python probe provider");
    let id = format!("sqlite-python-{}", uuid::Uuid::new_v4());
    let instance = provider.provision(VmSpec {
        id: VmId(id.clone()), root: RootFilesystem::Directory { host_path: root.to_owned() },
        disks: Vec::new(), mounts: Vec::new(), guest_volumes: Vec::new(),
        resources: VmResources { vcpus: 1, memory_mib: 256 }, network: NetworkMode::Disabled,
        command: GuestCommand {
            program: String::from("/usr/local/bin/python3"),
            args: vec![String::from("-c"), String::from("import sys,sqlite3,json,os; assert sys.version_info[:3]==(3,13,5); assert sqlite3.sqlite_version_info>=(3,24,0); print(json.dumps({'python':sys.version.split()[0],'sqlite':sqlite3.sqlite_version,'uid':os.getuid(),'gid':os.getgid()}),flush=True)")],
            env: BTreeMap::new(), working_dir: None,
        }, runtime_authority: None, runtime_git_bridge: None, private_http_service: None, labels: BTreeMap::new(),
    }).await.expect("actual Python probe VM");
    let pids = fs::read_to_string(
        PathBuf::from(env::var("HEPHAESTUS_LIBKRUN_CGROUP_ROOT").unwrap())
            .join(&id)
            .join("cgroup.procs"),
    )
    .expect("actual created probe cgroup");
    assert!(!pids.trim().is_empty());
    let mut events = instance.subscribe_events();
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        instance.start().await?;
        let collect = async {
            let mut output = Vec::new();
            while let Ok(event) = events.recv().await {
                match event {
                    VmEvent::Log { bytes, .. } => output.extend(bytes),
                    VmEvent::Exited(_) => break,
                    _ => {}
                }
            }
            output
        };
        let (exit, output) = tokio::join!(instance.wait(), collect);
        exit.map(|exit| (exit, output))
    })
    .await;
    instance
        .destroy()
        .await
        .expect("exact real Python probe destruction");
    drop(instance);
    drop(provider);
    assert_clean(&id);
    for pid in pids.split_whitespace() {
        assert!(
            !PathBuf::from(format!("/proc/{pid}")).exists(),
            "known probe PID retained"
        );
    }
    let (exit, output) = result
        .expect("bounded native Python proof")
        .expect("real probe start and exit");
    assert_eq!(exit.code, Some(0));
    let observed: serde_json::Value =
        serde_json::from_slice(&output).expect("actual guest Python JSON");
    assert_eq!(observed["python"], "3.13.5");
    assert_eq!(observed["uid"], 10001);
    assert_eq!(observed["gid"], 10001);
    let output = PathBuf::from(env::var("HEPHAESTUS_SQLITE_PUBLICATION_OUTPUT").unwrap());
    fs::write(
        output.join("guest-image-proof.json"),
        serde_json::to_vec_pretty(&observed).unwrap(),
    )
    .unwrap();
    println!(
        "REAL_SQLITE_IMAGE {observed} pids={} cleanup=confirmed",
        pids.trim()
    );
}

pub fn assert_clean(id: &str) {
    for variable in [
        "HEPHAESTUS_LIBKRUN_CGROUP_ROOT",
        "HEPHAESTUS_LIBKRUN_RUNTIME_ROOT",
    ] {
        let path = PathBuf::from(env::var(variable).unwrap()).join(id);
        assert!(
            !path.exists(),
            "created VM resource remains: {}",
            path.display()
        );
    }
}
