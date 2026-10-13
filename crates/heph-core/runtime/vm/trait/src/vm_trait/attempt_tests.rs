//! Default denial is not a provision/orphan-cleanup compatibility fallback.
use crate::{
    GuestCommand, NetworkMode, RootFilesystem, VmError, VmInstance, VmProvider, VmResources, VmSpec,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Legacy {
    provision: AtomicUsize,
    orphan: AtomicUsize,
}
#[async_trait::async_trait]
impl VmProvider for Legacy {
    fn name(&self) -> &'static str {
        "legacy-controlled"
    }
    async fn provision(&self, _spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provision.fetch_add(1, Ordering::SeqCst);
        Err(VmError::InvalidState("controlled no VM"))
    }
    async fn cleanup_orphan(&self, _id: &crate::VmId) -> Result<(), VmError> {
        self.orphan.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[test]
fn unsupported_attempt_preparation_never_provisions_or_uses_orphan_absence() {
    let provider = Legacy {
        provision: AtomicUsize::new(0),
        orphan: AtomicUsize::new(0),
    };
    let result = provider.prepare_provision_attempt(VmSpec {
        id: crate::VmId("controlled".into()),
        root: RootFilesystem::Directory {
            host_path: "/not-used".into(),
        },
        guest_volumes: Vec::new(),
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 256,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/bin/true".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: None,
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::new(),
    });
    assert!(
        matches!(result, Err(VmError::Unsupported { provider, .. }) if provider=="legacy-controlled")
    );
    assert_eq!(provider.provision.load(Ordering::SeqCst), 0);
    assert_eq!(provider.orphan.load(Ordering::SeqCst), 0);
}
