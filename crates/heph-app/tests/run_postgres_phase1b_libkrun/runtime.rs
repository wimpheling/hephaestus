use super::model::*;
use super::*;

pub struct StateSpecFactory {
    pub rootfs: PathBuf,
    pub rollback_release: ReleaseId,
    pub timeout_release: ReleaseId,
}

#[async_trait::async_trait]
impl VmSpecFactory for StateSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let (argument, timeout) = if run.release_id == self.rollback_release {
            ("--state-rollback", false)
        } else if run.release_id == self.timeout_release {
            ("--ignore-cancellation", true)
        } else {
            ("--state-only", false)
        };
        let mut labels = BTreeMap::new();
        if timeout {
            labels.insert(
                String::from("hephaestus.wall-clock-timeout-seconds"),
                String::from("1"),
            );
        }
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: self.rootfs.clone(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            // Exercise `passt` lifecycle cleanup as part of the full
            // acceptance scenario, without exposing any ingress port.
            network: NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            command: GuestCommand {
                program: String::from("/usr/libexec/hephaestus/integration-check"),
                args: vec![String::from(argument)],
                env: BTreeMap::from([
                    (
                        String::from("HEPH_RELEASE_MARKER"),
                        run.release_id.to_string(),
                    ),
                    (
                        String::from("HEPH_STATE_HOLD_MS"),
                        if run.kind == RunKind::Normal {
                            String::from("1000")
                        } else {
                            String::from("0")
                        },
                    ),
                ]),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels,
        })
    }
}

pub fn command(target: RunTarget) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: target.instance,
        instance_revision_id: target.revision,
        release_id: target.release,
        release_agent_id: target.release_agent,
        attachment_id: Some(target.attachment),
        kind: RunKind::Normal,
        requires_state: true,
    }
}

pub fn update_command(target: RunTarget) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: target.instance,
        instance_revision_id: target.revision,
        release_id: target.release,
        release_agent_id: target.release_agent,
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: true,
    }
}
