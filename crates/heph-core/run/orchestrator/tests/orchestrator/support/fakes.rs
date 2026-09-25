use async_trait::async_trait;
use run_domain::Run;
use run_orchestrator::{
    PreparedRunAuthority, PreparedRunSecrets, RunAuthorityError, RunAuthorityManager,
    RunAuthorizationError, RunLaunchAuthorizer, RunResourceObservationError, RunResourceObserver,
    RunSecretError, RunSecretManager, VmSpecFactory,
};
use runtime_types::RunId;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, RuntimeAuthorityBootstrap, VmError, VmId,
    VmResources, VmSpec,
};

use super::helpers::lock;

pub struct TestSpecFactory;

#[async_trait]
impl VmSpecFactory for TestSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/fake/root"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/bin/true"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels: BTreeMap::new(),
        })
    }
}

pub struct TimeoutSpecFactory;

#[async_trait]
impl VmSpecFactory for TimeoutSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let mut spec = TestSpecFactory.build(run).await?;
        spec.labels.insert(
            String::from("hephaestus.wall-clock-timeout-seconds"),
            String::from("1"),
        );
        Ok(spec)
    }
}

pub struct RecordingWorkspaceManager {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

pub struct RecordingRuntimeManager {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

pub struct RecordingRuntimeGitWorkspaceManager {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

pub struct RecordingAuthorityManager {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
    pub reject_acknowledgement: bool,
}

#[async_trait]
impl RunAuthorityManager for RecordingAuthorityManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        lock(&self.log).push("authority-prepare");
        Ok(PreparedRunAuthority {
            bootstrap: Some(RuntimeAuthorityBootstrap::new(
                run.id.as_uuid(),
                1,
                [0x5A; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
            )),
        })
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunAuthorityError> {
        lock(&self.log).push("authority-reauthorize");
        Ok(())
    }

    async fn acknowledge(
        &self,
        _run: &Run,
        _session_id: Uuid,
        _generation: u64,
    ) -> Result<(), RunAuthorityError> {
        lock(&self.log).push("authority-ack");
        if self.reject_acknowledgement {
            Err(RunAuthorityError::redacted(
                "exact issuance generation was not acknowledged",
            ))
        } else {
            Ok(())
        }
    }

    async fn revoke_after_guest(&self, _run_id: RunId) -> Result<(), RunAuthorityError> {
        lock(&self.log).push("authority-revoke");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        lock(&self.log).push("authority-recover");
        Ok(0)
    }
}

pub struct RecordingLaunchAuthorizer {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunLaunchAuthorizer for RecordingLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        lock(&self.log).push("authorize");
        Ok(())
    }
}

pub struct RecordingResourceObserver {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunResourceObserver for RecordingResourceObserver {
    async fn record(&self, run: &Run) -> Result<(), RunResourceObservationError> {
        assert_eq!(run.lease_fencing_token, Some(1));
        lock(&self.log).push("resource-recorded");
        Ok(())
    }
}

pub struct DenyLaunchAuthorizer;

#[async_trait]
impl RunLaunchAuthorizer for DenyLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        Err(RunAuthorizationError::redacted("denied by test policy"))
    }
}

pub struct RevocableLaunchAuthorizer {
    pub revoked: Arc<AtomicBool>,
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunLaunchAuthorizer for RevocableLaunchAuthorizer {
    async fn authorize(&self, _run: &Run) -> Result<(), RunAuthorizationError> {
        lock(&self.log).push("authorize");
        if self.revoked.load(Ordering::SeqCst) {
            Err(RunAuthorizationError::redacted("revoked by test policy"))
        } else {
            Ok(())
        }
    }
}

pub struct RevokeBeforeProvisionSecrets {
    pub log: Arc<StdMutex<Vec<&'static str>>>,
}

#[async_trait]
impl RunSecretManager for RevokeBeforeProvisionSecrets {
    async fn prepare(&self, _run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        lock(&self.log).push("secret-prepare");
        Ok(PreparedRunSecrets::default())
    }

    async fn reauthorize(&self, _run: &Run) -> Result<(), RunSecretError> {
        lock(&self.log).push("secret-reauthorize");
        Err(RunSecretError::redacted("revoked by test"))
    }

    async fn destroy_after_guest(&self, _run_id: RunId) -> Result<(), RunSecretError> {
        lock(&self.log).push("secret-destroy");
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunSecretError> {
        Ok(0)
    }
}
