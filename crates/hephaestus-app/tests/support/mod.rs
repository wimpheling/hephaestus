//! Shared daemon integration-test fixtures.

use async_trait::async_trait;
use hephaestus_app::VmBackendConfig;
use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{broadcast, watch};
use vm_libkrun::LibkrunConfig;
use vm_trait::{StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec};

/// The VM backend and storage paths a daemon integration test needs.
pub struct BackendFixture {
    /// Backend configured for either the lightweight result guest or libkrun.
    pub(crate) backend: VmBackendConfig,
    /// Root image or directory used by the test's release attachment.
    pub(crate) root_image: PathBuf,
    /// Per-test volume root.
    pub(crate) volume_root: PathBuf,
    /// Runtime roots that the caller must verify after shutdown.
    pub(crate) transient_runtime_roots: Vec<PathBuf>,
}

/// Creates the backend used by daemon integration tests.
pub async fn backend_fixture(temporary_root: &Path) -> BackendFixture {
    if env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() == Ok("1") {
        let runtime_root = required_path("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT");
        let image_root = required_path("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
        let root_image = required_path("HEPHAESTUS_LIBKRUN_ROOTFS");
        let disk_root = required_path("HEPHAESTUS_LIBKRUN_DISK_ROOT");
        let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
        let worker = required_path("HEPHAESTUS_LIBKRUN_WORKER");
        let cgroup_root = required_path("HEPHAESTUS_LIBKRUN_CGROUP_ROOT");
        let volume_root = disk_root.join(format!("app-golden-volumes-{}", uuid::Uuid::new_v4()));
        let provider = LibkrunConfig::new(
            runtime_root.clone(),
            vec![image_root],
            vec![disk_root],
            vec![mount_root, temporary_root.to_path_buf()],
            worker,
            cgroup_root,
        );
        BackendFixture {
            backend: VmBackendConfig::Libkrun(Box::new(provider)),
            root_image,
            volume_root,
            transient_runtime_roots: vec![runtime_root],
        }
    } else {
        let root_image = temporary_root.join("root-image");
        tokio::fs::create_dir(&root_image)
            .await
            .expect("root image directory");
        BackendFixture {
            backend: VmBackendConfig::Custom(Arc::new(ResultGuestProvider)),
            root_image,
            volume_root: temporary_root.join("volumes"),
            transient_runtime_roots: Vec::new(),
        }
    }
}

struct ResultGuestProvider;

#[async_trait]
impl VmProvider for ResultGuestProvider {
    fn name(&self) -> &'static str {
        "golden-result-guest"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(ResultGuestInstance::new(spec)?))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct ResultGuestInstance {
    id: VmId,
    run_work: Option<PathBuf>,
    build_output: Option<PathBuf>,
    runtime_authority: Option<(uuid::Uuid, u64)>,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl ResultGuestInstance {
    fn new(spec: VmSpec) -> Result<Self, VmError> {
        let run_source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-source");
        let run_work = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work");
        let run_work = match (run_source, run_work) {
            (Some(source), Some(work)) => {
                if !source.read_only {
                    return Err(VmError::InvalidSpec {
                        field: String::from("mounts"),
                        reason: String::from("repository source mount is writable"),
                    });
                }
                if work.read_only {
                    return Err(VmError::InvalidSpec {
                        field: String::from("mounts"),
                        reason: String::from("repository work mount is read-only"),
                    });
                }
                Some(work.host_path.clone())
            }
            (None, None) => None,
            _ => {
                return Err(VmError::InvalidSpec {
                    field: String::from("mounts"),
                    reason: String::from("repository source and work mounts must be paired"),
                });
            }
        };
        let build_source = spec.mounts.iter().find(|mount| mount.tag == "build-source");
        let build_output = spec.mounts.iter().find(|mount| mount.tag == "build-output");
        let build_output = match (build_source, build_output) {
            (Some(source), Some(output)) => {
                if !source.read_only {
                    return Err(VmError::InvalidSpec {
                        field: String::from("mounts"),
                        reason: String::from("build source mount is writable"),
                    });
                }
                if output.read_only {
                    return Err(VmError::InvalidSpec {
                        field: String::from("mounts"),
                        reason: String::from("build output mount is read-only"),
                    });
                }
                Some(output.host_path.clone())
            }
            (None, None) => None,
            _ => {
                return Err(VmError::InvalidSpec {
                    field: String::from("mounts"),
                    reason: String::from("build source and output mounts must be paired"),
                });
            }
        };
        if run_work.is_some() && build_output.is_some() {
            return Err(VmError::InvalidSpec {
                field: String::from("mounts"),
                reason: String::from("run and build mounts cannot be combined"),
            });
        }
        let runtime_authority = spec
            .runtime_authority
            .as_ref()
            .map(|authority| (authority.session_id(), authority.generation()));
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Ok(Self {
            id: spec.id,
            run_work,
            build_output,
            runtime_authority,
            events,
            exit,
        })
    }
}

#[async_trait]
impl VmInstance for ResultGuestInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        let _started = self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        });
        if let Some((session_id, generation)) = self.runtime_authority {
            let _acknowledged = self.events.send(VmEvent::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            });
        }
        let _ready = self.events.send(VmEvent::Ready);
        if let Some(work) = &self.run_work {
            tokio::fs::write(work.join("input.txt"), "agent edit\n")
                .await
                .map_err(test_vm_error)?;
            tokio::fs::write(work.join("reports/result.txt"), "durable report\n")
                .await
                .map_err(test_vm_error)?;
        }
        if let Some(output) = &self.build_output {
            let binary = output.join("bin");
            tokio::fs::create_dir_all(&binary)
                .await
                .map_err(test_vm_error)?;
            let artifact = binary.join("golden");
            tokio::fs::write(&artifact, "#!/bin/sh\nexit 0\n")
                .await
                .map_err(test_vm_error)?;
            let mut permissions = tokio::fs::metadata(&artifact)
                .await
                .map_err(test_vm_error)?
                .permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
            tokio::fs::set_permissions(artifact, permissions)
                .await
                .map_err(test_vm_error)?;
        }
        let _finalize = self.events.send(VmEvent::FinalizeResult {
            message: String::from("golden agent result"),
        });
        let exit = VmExit {
            code: Some(0),
            signal: None,
        };
        let _exited = self.events.send(VmEvent::Exited(exit.clone()));
        self.exit.send_replace(Some(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current = receiver.borrow_and_update().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("golden result guest exited"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn test_vm_error(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("golden-result-guest"),
        code: String::from("workspace-write"),
        source: Box::new(error),
    }
}

/// Reads a required libkrun E2E path from the environment.
pub fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} is required for libkrun golden E2E"),
        PathBuf::from,
    )
}
