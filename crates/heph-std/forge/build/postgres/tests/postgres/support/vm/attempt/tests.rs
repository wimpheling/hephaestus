//! Real fixture token/output checks; no PostgreSQL/native guest claim.
use super::super::OutputProvider;
use super::RetainedError;
use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, VmError, VmMount, VmProvider, VmResources, VmSpec,
};

struct Fixture {
    temporary: tempfile::TempDir,
    provider: OutputProvider,
}
impl Fixture {
    fn new(fail: bool) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        for name in ["source", "output"] {
            fs::create_dir(temporary.path().join(name)).unwrap();
        }
        Self {
            temporary,
            provider: OutputProvider {
                provisions: Arc::new(AtomicUsize::new(0)),
                fail_next_provision: Arc::new(AtomicBool::new(fail)),
            },
        }
    }
    fn spec(&self) -> VmSpec {
        VmSpec {
            id: vm_trait::VmId("original-output".into()),
            root: RootFilesystem::Directory {
                host_path: self.temporary.path().to_owned(),
            },
            guest_volumes: Vec::new(),
            disks: Vec::new(),
            mounts: vec![
                VmMount {
                    tag: "build-source".into(),
                    host_path: self.temporary.path().join("source"),
                    guest_path: "/source".into(),
                    read_only: true,
                },
                VmMount {
                    tag: "build-output".into(),
                    host_path: self.temporary.path().join("output"),
                    guest_path: "/output".into(),
                    read_only: false,
                },
            ],
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: "/fixture".into(),
                args: Vec::new(),
                env: BTreeMap::from([("HEPH_BUILD_GUEST".into(), "1".into())]),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels: BTreeMap::new(),
        }
    }
}
fn retained(error: &VmError) -> &Arc<VmError> {
    let VmError::Provider { source, .. } = error else {
        panic!("original fixture error expected");
    };
    &source.downcast_ref::<RetainedError>().unwrap().0
}
#[tokio::test]
async fn concurrent_duplicate_returns_same_original_and_cleanup_preserves_output() {
    let fixture = Fixture::new(false);
    let attempt = fixture
        .provider
        .prepare_provision_attempt(fixture.spec())
        .unwrap();
    assert_eq!(fixture.provider.provisions.load(Ordering::SeqCst), 0);
    let (first, duplicate) = tokio::join!(attempt.provision(), attempt.provision());
    let first = first.unwrap();
    let duplicate = duplicate.unwrap();
    assert!(Arc::ptr_eq(&first, &duplicate));
    assert_eq!(fixture.provider.provisions.load(Ordering::SeqCst), 1);
    first.start().await.unwrap();
    let output = fixture.temporary.path().join("output/bin/agent");
    assert_eq!(fs::read(&output).unwrap(), b"immutable built executable");
    attempt.cleanup().await.unwrap();
    attempt.cleanup().await.unwrap();
    assert!(matches!(attempt.provision().await, Err(VmError::Destroyed)));
    assert_eq!(fixture.provider.provisions.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(&output).unwrap(), b"immutable built executable");
    assert!(fixture.temporary.path().join("source").is_dir());
}
#[tokio::test]
async fn unused_cleanup_closes_without_any_fixture_provision_effect() {
    let fixture = Fixture::new(true);
    let attempt = fixture
        .provider
        .prepare_provision_attempt(fixture.spec())
        .unwrap();
    attempt.cleanup().await.unwrap();
    assert!(matches!(attempt.provision().await, Err(VmError::Destroyed)));
    attempt.cleanup().await.unwrap();
    assert_eq!(fixture.provider.provisions.load(Ordering::SeqCst), 0);
    assert!(fixture.provider.fail_next_provision.load(Ordering::SeqCst));
    assert_eq!(
        fs::read_dir(fixture.temporary.path().join("output"))
            .unwrap()
            .count(),
        0
    );
}
#[tokio::test]
async fn intentional_pre_instance_failure_is_retained_without_replacement_birth() {
    let fixture = Fixture::new(true);
    let attempt = fixture
        .provider
        .prepare_provision_attempt(fixture.spec())
        .unwrap();
    assert!(fixture.provider.fail_next_provision.load(Ordering::SeqCst));
    let Err(first) = attempt.provision().await else {
        panic!("intentional fixture failure expected");
    };
    assert!(!fixture.provider.fail_next_provision.load(Ordering::SeqCst));
    fixture
        .provider
        .fail_next_provision
        .store(true, Ordering::SeqCst);
    let Err(duplicate) = attempt.provision().await else {
        panic!("failure cannot reprovision");
    };
    assert!(Arc::ptr_eq(retained(&first), retained(&duplicate)));
    assert!(
        matches!(retained(&first).as_ref(), VmError::Provider { code, .. } if code=="fixture-io")
    );
    assert_eq!(fixture.provider.provisions.load(Ordering::SeqCst), 1);
    assert!(fixture.provider.fail_next_provision.load(Ordering::SeqCst));
    attempt.cleanup().await.unwrap();
    assert!(matches!(attempt.provision().await, Err(VmError::Destroyed)));
    assert_eq!(
        fs::read_dir(fixture.temporary.path().join("output"))
            .unwrap()
            .count(),
        0
    );
}
