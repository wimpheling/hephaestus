use super::vm_spec_factory::invalid_spec;
use super::{
    Arc, BTreeMap, PathBuf, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmMetric,
    VmProvider, VmSpec,
};
use async_trait::async_trait;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;
pub struct ResultFixtureProvider;

#[async_trait]
impl VmProvider for ResultFixtureProvider {
    fn name(&self) -> &'static str {
        "local-result-fixture"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(ResultFixtureInstance::new(spec)?))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct ResultFixtureInstance {
    id: VmId,
    work: Option<PathBuf>,
    output: Option<PathBuf>,
    exit_code: i32,
    uncertain_exit: bool,
    runtime_authority: Option<(Uuid, u64)>,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl ResultFixtureInstance {
    fn new(spec: VmSpec) -> Result<Self, VmError> {
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-source");
        let work = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work");
        let work = match (source, work) {
            (Some(source), Some(work)) => {
                if !source.read_only {
                    return Err(invalid_spec(
                        "mounts",
                        "repository source mount must be read-only",
                    ));
                }
                if work.read_only {
                    return Err(invalid_spec(
                        "mounts",
                        "repository work mount must be writable",
                    ));
                }
                Some(work.host_path.clone())
            }
            (None, None) => None,
            _ => {
                return Err(invalid_spec(
                    "mounts",
                    "repository source and work mounts must be paired",
                ));
            }
        };
        let output = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-output")
            .map(|mount| mount.host_path.clone());
        let exit_code = if spec.command.args.iter().any(|value| value == "fail") {
            23
        } else {
            0
        };
        let uncertain_exit = spec.command.args.iter().any(|value| value == "uncertain");
        let runtime_authority = spec
            .runtime_authority
            .as_ref()
            .map(|authority| (authority.session_id(), authority.generation()));
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Ok(Self {
            id: spec.id,
            work,
            output,
            exit_code,
            uncertain_exit,
            runtime_authority,
            events,
            exit,
        })
    }
}

#[async_trait]
impl VmInstance for ResultFixtureInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        drop(self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        }));
        if let Some((session_id, generation)) = self.runtime_authority {
            drop(self.events.send(VmEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            }));
        }
        drop(self.events.send(VmEvent::Ready));
        if let Some(work) = &self.work {
            tokio::fs::write(
                work.join("input.txt"),
                "agent reviewed and changed this file\n",
            )
            .await
            .map_err(fixture_vm_error)?;
            let reports = work.join("reports");
            tokio::fs::create_dir_all(&reports)
                .await
                .map_err(fixture_vm_error)?;
            tokio::fs::write(reports.join("result.txt"), "durable browser E2E report\n")
                .await
                .map_err(fixture_vm_error)?;
        }
        if let Some(output) = &self.output {
            let reports = output.join("reports");
            tokio::fs::create_dir_all(&reports)
                .await
                .map_err(fixture_vm_error)?;
            tokio::fs::write(reports.join("result.txt"), "built browser artifact\n")
                .await
                .map_err(fixture_vm_error)?;
        }
        drop(self.events.send(VmEvent::Log {
            stream: heph_runtime::LogStream::Stdout,
            bytes: b"fixture agent completed workspace edits\n".to_vec(),
        }));
        drop(self.events.send(VmEvent::Metric(VmMetric {
            name: String::from("fixture.cpu_ms"),
            value: 42.0,
            labels: BTreeMap::from([(String::from("phase"), String::from("result"))]),
        })));
        if self.work.is_some() {
            drop(self.events.send(VmEvent::FinalizeResult {
                message: String::from("fixture agent result"),
            }));
        }
        let exit = VmExit {
            code: (!self.uncertain_exit).then_some(self.exit_code),
            signal: self.uncertain_exit.then_some(9),
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
            let current_exit = receiver.borrow_and_update().clone();
            if let Some(exit) = current_exit {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("fixture guest exited without a result"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn fixture_vm_error(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("local-result-fixture"),
        code: String::from("workspace-write"),
        source: Box::new(error),
    }
}
