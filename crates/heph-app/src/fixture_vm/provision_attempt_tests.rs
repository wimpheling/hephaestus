//! Actual memory constructor/attempt controls, not browser or VM/native proof.
use super::*;
use crate::fixture_vm::ResultFixtureProvider;
use std::{collections::BTreeMap, fs, os::unix::fs::MetadataExt, path::Path};
use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, RootFilesystem, VmDisk, VmId, VmMount, VmProvider,
    VmResources,
};

fn fixture() -> (tempfile::TempDir, VmSpec) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in ["source", "work", "output"] {
        fs::create_dir(root.join(name)).unwrap();
    }
    fs::write(root.join("source/original.txt"), b"caller source").unwrap();
    fs::write(root.join("backing.raw"), b"caller backing").unwrap();
    let mounts = [
        ("repository-source", "source", "/repository-source", true),
        ("repository-work", "work", "/repository-work", false),
        ("build-output", "output", "/build-output", false),
    ]
    .into_iter()
    .map(|(tag, host, guest, read_only)| VmMount {
        tag: tag.into(),
        host_path: root.join(host),
        guest_path: guest.into(),
        read_only,
    })
    .collect();
    let spec = VmSpec {
        id: VmId("browser-original-controlled".into()),
        root: RootFilesystem::Directory {
            host_path: root.to_owned(),
        },
        disks: vec![VmDisk {
            id: "caller".into(),
            host_path: root.join("backing.raw"),
            format: DiskFormat::Raw,
            read_only: false,
        }],
        guest_volumes: Vec::new(),
        mounts,
        resources: VmResources {
            vcpus: 2,
            memory_mib: 256,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/fixture-only".into(),
            args: vec!["fail".into()],
            env: BTreeMap::from([("controlled".into(), "exact".into())]),
            working_dir: Some("/work".into()),
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::from([("original".into(), "retained".into())]),
    };
    (directory, spec)
}
fn untouched(root: &Path) {
    assert_eq!(
        fs::read(root.join("source/original.txt")).unwrap(),
        b"caller source"
    );
    assert_eq!(
        fs::read(root.join("backing.raw")).unwrap(),
        b"caller backing"
    );
    assert!(!root.join("work/input.txt").exists());
    assert!(!root.join("work/reports").exists());
    assert!(!root.join("output/reports").exists());
}
fn error_source(error: &VmError) -> &Arc<VmError> {
    let VmError::Provider {
        provider,
        code,
        source,
    } = error
    else {
        panic!("cached original constructor error required");
    };
    assert_eq!(provider, "local-result-fixture");
    assert_eq!(code, "original-attempt");
    &source.downcast_ref::<RetainedError>().unwrap().0
}

#[tokio::test]
async fn unused_attempt_close_denies_provision_and_preserves_caller_paths() {
    let (directory, spec) = fixture();
    let attempt = ResultFixtureProvider
        .prepare_provision_attempt(spec)
        .unwrap();
    untouched(directory.path()); // Actual provider method prepares without IO.
    attempt.cleanup().await.unwrap();
    attempt.cleanup().await.unwrap();
    assert!(matches!(attempt.provision().await, Err(VmError::Destroyed)));
    untouched(directory.path());
}

#[tokio::test]
async fn repeated_provision_retains_full_spec_and_same_original_instance() {
    let (directory, spec) = fixture();
    let id = spec.id.clone();
    let full_spec = format!("{spec:?}"); // Controlled values, no runtime credential.
    let descriptor = fs::File::open(directory.path().join("backing.raw")).unwrap();
    let before = descriptor.metadata().unwrap();
    let attempt = Attempt::new(spec);
    assert_eq!(format!("{:?}", attempt.spec), full_spec);
    assert!(!attempt.state.try_lock().unwrap().started);
    let first = attempt.provision().await.unwrap();
    let second = attempt.provision().await.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.id(), &id);
    let state = attempt.state.lock().await;
    assert!(state.started && !state.cleaned);
    let Some(Ok(cached)) = &state.outcome else {
        panic!("same original instance cache missing");
    };
    assert!(Arc::ptr_eq(&first, cached));
    drop(state);
    assert_eq!(format!("{:?}", attempt.spec), full_spec);
    attempt.cleanup().await.unwrap();
    attempt.cleanup().await.unwrap();
    assert!(attempt.state.try_lock().unwrap().cleaned);
    assert!(matches!(attempt.provision().await, Err(VmError::Destroyed)));
    untouched(directory.path());
    let after = descriptor.metadata().unwrap();
    let named = fs::metadata(directory.path().join("backing.raw")).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!((before.dev(), before.ino()), (named.dev(), named.ino()));
    // start (the unchanged actual workspace writer) was never invoked here.
}

#[tokio::test]
async fn real_constructor_failure_is_cached_and_cleanup_closes_without_birth() {
    let (directory, mut spec) = fixture();
    spec.mounts.retain(|mount| mount.tag != "repository-work");
    let full_spec = format!("{spec:?}");
    let attempt = Attempt::new(spec);
    let first = attempt.provision().await.err().unwrap();
    let second = attempt.provision().await.err().unwrap();
    assert!(Arc::ptr_eq(error_source(&first), error_source(&second)));
    assert!(
        matches!(error_source(&first).as_ref(), VmError::InvalidSpec { field, reason }
        if field == "mounts" && reason == "repository source and work mounts must be paired")
    );
    let state = attempt.state.lock().await;
    assert!(state.started && !state.cleaned);
    let Some(Err(cached)) = &state.outcome else {
        panic!("original failure cache missing");
    };
    assert!(Arc::ptr_eq(cached, error_source(&first)));
    drop(state);
    assert_eq!(format!("{:?}", attempt.spec), full_spec);
    attempt.cleanup().await.unwrap();
    assert!(attempt.state.try_lock().unwrap().cleaned);
    assert!(matches!(attempt.provision().await, Err(VmError::Destroyed)));
    untouched(directory.path());
}
