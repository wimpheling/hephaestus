//! Controlled exact persisted ports; no `PostgreSQL` lineage/permission proof.
use async_trait::async_trait;
use run_domain::{Run, RunKind, RunState};
use run_orchestrator::{
    PreparedRunAuthority, PreparedRunRuntime, PreparedRunSecrets, RunAuthorityError,
    RunAuthorityManager, RunAuthorizationError, RunLaunchAuthorizer, RunRepository,
    RunRuntimeError, RunRuntimeManager, RunSecretError, RunSecretManager,
};
use runtime_types::RunId;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
use vm_trait::RuntimeAuthorityBootstrap;

use crate::{
    canonical::Fixture,
    support::{MemoryRepository, lock},
};

pub struct Ports {
    runs: Arc<MemoryRepository>,
    log: Arc<std::sync::Mutex<Vec<&'static str>>>,
    pub deny_launch: AtomicBool,
    pub deny_authority: AtomicBool,
    pub deny_secrets: AtomicBool,
    pub delayed: AtomicBool,
}
impl Ports {
    pub fn new(fixture: &Fixture) -> Self {
        Self {
            runs: fixture.runs.clone(),
            log: fixture.log.clone(),
            deny_launch: AtomicBool::new(false),
            deny_authority: AtomicBool::new(false),
            deny_secrets: AtomicBool::new(false),
            delayed: AtomicBool::new(false),
        }
    }
    async fn exact(&self, run: &Run) {
        assert_eq!(run, &self.runs.get(run.id).await.unwrap());
        if run.kind == RunKind::Invocation {
            assert!(self.runs.invocation_admitted.load(Ordering::SeqCst));
            let plan = self.runs.planned_vm.lock().await;
            assert_eq!(
                plan.as_ref().unwrap().1.0.as_str(),
                run.vm_id.as_deref().unwrap()
            );
            drop(plan);
        }
    }
    async fn delay(&self) {
        if self.delayed.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
    }
}
#[async_trait]
impl RunLaunchAuthorizer for Ports {
    async fn authorize(&self, _: &Run) -> Result<(), RunAuthorizationError> {
        panic!("complete-set route used scalar launch authorization");
    }
    async fn authorize_with_volumes(&self, run: &Run) -> Result<(), RunAuthorizationError> {
        self.exact(run).await;
        lock(&self.log).push(if run.state == RunState::LeasingVolume {
            "first-launch"
        } else {
            "later-launch"
        });
        self.delay().await;
        if self.deny_launch.load(Ordering::SeqCst) {
            return Err(RunAuthorizationError::redacted(
                "controlled current source/execute withdrawal",
            ));
        }
        Ok(())
    }
}
#[async_trait]
impl RunAuthorityManager for Ports {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        self.exact(run).await;
        assert_eq!(run.state, RunState::Provisioning);
        lock(&self.log).push("first-authority-prepare");
        Ok(PreparedRunAuthority {
            bootstrap: Some(RuntimeAuthorityBootstrap::new(
                run.id.as_uuid(),
                1,
                [0x5A; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
            )),
        })
    }
    async fn reauthorize(&self, run: &Run) -> Result<(), RunAuthorityError> {
        self.exact(run).await;
        lock(&self.log).push(if run.state == RunState::LeasingVolume {
            "first-authority"
        } else {
            "later-authority"
        });
        self.delay().await;
        if self.deny_authority.load(Ordering::SeqCst) {
            return Err(RunAuthorityError::redacted(
                "controlled first-acquisition facade refusal",
            ));
        }
        Ok(())
    }
    async fn acknowledge(
        &self,
        run: &Run,
        session: Uuid,
        generation: u64,
    ) -> Result<(), RunAuthorityError> {
        self.exact(run).await;
        assert_eq!(run.state, RunState::Provisioning);
        assert_eq!((session, generation), (run.id.as_uuid(), 1));
        lock(&self.log).push("first-ack");
        Ok(())
    }
    async fn revoke_after_guest(&self, _: RunId) -> Result<(), RunAuthorityError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunAuthorityError> {
        Ok(0)
    }
}
#[async_trait]
impl RunSecretManager for Ports {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        self.exact(run).await;
        assert_eq!(run.state, RunState::Provisioning);
        lock(&self.log).push("first-secret-prepare");
        Ok(PreparedRunSecrets::default())
    }
    async fn reauthorize(&self, run: &Run) -> Result<(), RunSecretError> {
        self.exact(run).await;
        lock(&self.log).push(if run.state == RunState::LeasingVolume {
            "leasing-secret"
        } else {
            "provisioning-secret"
        });
        if (run.kind == RunKind::Invocation && run.state == RunState::LeasingVolume)
            || self.deny_secrets.load(Ordering::SeqCst)
        {
            return Err(RunSecretError::redacted(
                "controlled phaseful saved-empty verifier refusal",
            ));
        }
        if run.kind == RunKind::Invocation {
            assert!(matches!(
                run.state,
                RunState::Provisioning | RunState::Starting | RunState::Running
            ));
        }
        Ok(())
    }
    async fn destroy_after_guest(&self, _: RunId) -> Result<(), RunSecretError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunSecretError> {
        Ok(0)
    }
}
#[async_trait]
impl RunRuntimeManager for Ports {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        self.exact(run).await;
        assert_eq!(run.state, RunState::Provisioning);
        lock(&self.log).push("first-runtime");
        Ok(PreparedRunRuntime::default())
    }
    async fn destroy(&self, _: RunId) -> Result<(), RunRuntimeError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
}
