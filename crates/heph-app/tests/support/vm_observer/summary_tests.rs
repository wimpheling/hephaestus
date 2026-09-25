use super::super::{
    CommandClass, NetworkSummary, VmSpecObserver, VmSpecSummary, selected_summaries,
};
use super::summarize_spec;
use async_trait::async_trait;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tempfile::TempDir;
use vm_fake::FakeProvider;
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmInstance, VmProvider, VmResources,
    VmSpec,
};

fn spec(root: &TempDir) -> VmSpec {
    VmSpec {
        id: VmId(String::from("00000000-0000-0000-0000-000000000001")),
        root: RootFilesystem::Directory {
            host_path: root.path().to_owned(),
        },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 64,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: String::from("/bin/sh"),
            args: vec![String::from("build.sh")],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::from([(String::from("hephaestus.kind"), String::from("build"))]),
    }
}

#[test]
fn requires_nonempty_fixture_patterns() {
    assert!(VmSpecObserver::new(Arc::new(FakeProvider::new()), Vec::new()).is_err());
    assert!(VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![Vec::new()]).is_err());
}

#[tokio::test]
async fn captures_safe_summary_and_forwards_to_real_provider() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
        .expect("patterns");
    observer.provision(spec(&root)).await.expect("provision");
    let summaries = observer.summaries();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].kind, "build");
    assert_eq!(summaries[0].network, NetworkSummary::Disabled);
    assert_eq!(summaries[0].command.class, CommandClass::Build);
}

#[tokio::test]
async fn rejects_missing_mount_without_disclosing_path() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
        .expect("patterns");
    let mut invalid = spec(&root);
    invalid.mounts.push(vm_trait::VmMount {
        tag: String::from("bad"),
        host_path: root.path().join("missing-host-path"),
        guest_path: PathBuf::from("/unexpected/guest-path"),
        read_only: true,
    });
    let error = match observer.provision(invalid).await {
        Ok(_) => panic!("missing mount was accepted"),
        Err(error) => error,
    };
    assert!(!error.to_string().contains("missing-host-path"));
    assert!(observer.summaries().is_empty());
}

#[tokio::test]
async fn rejects_network_contract_and_fixture_pattern_before_forwarding() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
        .expect("patterns");
    let mut bad_network = spec(&root);
    bad_network.network = NetworkMode::UserMode {
        ingress: Vec::new(),
    };
    bad_network
        .labels
        .insert(String::from("fixture-label"), String::from("ok"));
    observer
        .provision(bad_network)
        .await
        .expect("observer records before assertion");
    assert_eq!(
        observer.summaries()[0].network,
        NetworkSummary::UserMode { ingress_count: 0 }
    );
    assert!(
        observer
            .assert_build_contract(&[String::from("00000000-0000-0000-0000-000000000001",)])
            .is_err()
    );

    let mut bad_value = spec(&root);
    bad_value.command.args = vec![String::from("fixture-value")];
    let error = match observer.provision(bad_value).await {
        Ok(_) => panic!("fixture pattern was accepted"),
        Err(error) => error,
    };
    assert!(!error.to_string().contains("fixture-value"));
}

struct FailingProvider;

#[async_trait]
impl VmProvider for FailingProvider {
    fn name(&self) -> &'static str {
        "failing"
    }
    async fn provision(&self, _: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Err(VmError::Unavailable {
            resource: String::from("synthetic"),
            reason: String::from("forwarded"),
        })
    }
    async fn cleanup_orphan(&self, _: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

#[tokio::test]
async fn preserves_forwarding_error() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FailingProvider), vec![b"fixture".to_vec()])
        .expect("patterns");
    let error = match observer.provision(spec(&root)).await {
        Ok(_) => panic!("forwarding error was lost"),
        Err(error) => error,
    };
    assert!(matches!(error, VmError::Unavailable { resource, .. } if resource == "synthetic"));
}

#[tokio::test]
async fn rejects_extra_disk_and_writable_source_from_contract_summary() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
        .expect("patterns");
    let mut invalid = spec(&root);
    invalid.mounts.push(vm_trait::VmMount {
        tag: String::from("source"),
        host_path: root.path().to_owned(),
        guest_path: PathBuf::from("/workspace/source"),
        read_only: false,
    });
    let disk = root.path().join("extra.raw");
    std::fs::write(&disk, b"disk").expect("disk");
    invalid.disks.push(vm_trait::VmDisk {
        id: String::from("foreign-disk"),
        host_path: disk,
        format: vm_trait::DiskFormat::Raw,
        read_only: false,
    });
    observer.provision(invalid).await.expect("record");
    assert!(
        observer
            .assert_build_contract(&[String::from("00000000-0000-0000-0000-000000000001",)])
            .is_err()
    );
}

fn selected_summary(root: &TempDir, id: &str) -> VmSpecSummary {
    let mut summary = summarize_spec(&spec(root), &[b"fixture".to_vec()])
        .expect("valid synthetic VM specification");
    summary.vm_id = id.to_owned();
    summary
}

#[test]
fn selects_each_expected_id_once() {
    let root = TempDir::new().expect("root");
    let summaries = [selected_summary(&root, "a"), selected_summary(&root, "b")];
    let selected = selected_summaries(&summaries, &[String::from("a"), String::from("b")])
        .expect("each selected ID is observed exactly once");
    assert_eq!(selected.len(), 2);
}

#[test]
fn rejects_duplicate_selected_id_when_another_id_is_missing() {
    let root = TempDir::new().expect("root");
    let summaries = [selected_summary(&root, "a"), selected_summary(&root, "a")];
    assert!(
        selected_summaries(&summaries, &[String::from("a"), String::from("b")],).is_err(),
        "a duplicate selected VM must not satisfy a missing selected ID"
    );
}

#[test]
fn rejects_duplicate_expected_id() {
    let root = TempDir::new().expect("root");
    let summaries = [selected_summary(&root, "a")];
    assert!(selected_summaries(&summaries, &[String::from("a"), String::from("a")],).is_err());
}

#[tokio::test]
async fn rejects_fixture_pattern_in_environment_key() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
        .expect("patterns");
    let mut invalid = spec(&root);
    invalid
        .command
        .env
        .insert(String::from("fixture-key"), String::from("value"));
    let error = match observer.provision(invalid).await {
        Ok(_) => panic!("fixture environment key was accepted"),
        Err(error) => error,
    };
    assert!(!error.to_string().contains("fixture-key"));
}

#[tokio::test]
async fn rejects_fixture_pattern_in_vm_identifier() {
    let root = TempDir::new().expect("root");
    let observer = VmSpecObserver::new(Arc::new(FakeProvider::new()), vec![b"fixture".to_vec()])
        .expect("patterns");
    let mut invalid = spec(&root);
    invalid.id = VmId(String::from("fixture-vm-id"));
    let error = match observer.provision(invalid).await {
        Ok(_) => panic!("fixture pattern in VM ID was accepted"),
        Err(error) => error,
    };
    assert!(!error.to_string().contains("fixture-vm-id"));
    assert!(observer.summaries().is_empty());
}
