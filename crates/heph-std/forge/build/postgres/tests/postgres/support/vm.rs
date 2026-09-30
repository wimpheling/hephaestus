//! Deterministic VM provider fixture for isolated builds.

use async_trait::async_trait;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{broadcast, watch};
use vm_trait::{
    LogStream, StopMode, VmError, VmEvent, VmExit, VmInstance, VmMetric, VmProvider, VmSpec,
};

pub struct OutputProvider {
    pub provisions: Arc<AtomicUsize>,
    pub fail_next_provision: Arc<AtomicBool>,
}

#[async_trait]
impl VmProvider for OutputProvider {
    fn name(&self) -> &'static str {
        "isolated-build-test"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provisions.fetch_add(1, Ordering::SeqCst);
        if self.fail_next_provision.swap(false, Ordering::SeqCst) {
            return Err(vm_io(std::io::Error::other(
                "intentional provision failure",
            )));
        }
        assert!(spec.disks.is_empty());
        assert_eq!(
            spec.command.env.get("HEPH_BUILD_GUEST").map(String::as_str),
            Some("1")
        );
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-source")
            .expect("source mount");
        assert!(source.read_only);
        assert!(!source.host_path.join(".git").exists());
        let output = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-output")
            .expect("output mount");
        assert!(!output.read_only);
        Ok(Arc::new(OutputInstance::new(
            spec.id,
            output.host_path.clone(),
        )))
    }

    async fn cleanup_orphan(&self, _id: &vm_trait::VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct OutputInstance {
    id: vm_trait::VmId,
    output: PathBuf,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl OutputInstance {
    fn new(id: vm_trait::VmId, output: PathBuf) -> Self {
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Self {
            id,
            output,
            events,
            exit,
        }
    }
}

#[async_trait]
impl VmInstance for OutputInstance {
    fn id(&self) -> &vm_trait::VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        fs::create_dir(self.output.join("bin")).map_err(vm_io)?;
        let executable = self.output.join("bin/agent");
        fs::write(&executable, b"immutable built executable").map_err(vm_io)?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).map_err(vm_io)?;
        drop(self.events.send(VmEvent::Log {
            stream: LogStream::Stdout,
            bytes: b"build completed".to_vec(),
        }));
        drop(self.events.send(VmEvent::Metric(VmMetric {
            name: String::from("build.outputs"),
            value: 1.0,
            labels: BTreeMap::new(),
        })));
        let exit = VmExit {
            code: Some(0),
            signal: None,
        };
        drop(self.events.send(VmEvent::Exited(exit.clone())));
        self.exit.send_replace(Some(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current = receiver.borrow().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver.changed().await.map_err(|_| VmError::Destroyed)?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn vm_io(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("isolated-build-test"),
        code: String::from("fixture-io"),
        source: Box::new(error),
    }
}
