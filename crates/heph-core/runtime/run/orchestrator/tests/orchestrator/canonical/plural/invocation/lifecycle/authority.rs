//! Controlled current-run port assertions; not `PostgreSQL` admission proof.
use async_trait::async_trait;
use run_domain::{Run, RunState};
use run_orchestrator::{
    PreparedRunAuthority, PreparedRunRuntime, PreparedRunSecrets, RunAuthorityError,
    RunAuthorityManager, RunRepository, RunRuntimeError, RunRuntimeManager, RunSecretError,
    RunSecretManager,
};
use runtime_types::RunId;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use uuid::Uuid;
use vm_trait::RuntimeAuthorityBootstrap;

use crate::canonical::Fixture;
use crate::canonical::repository::CleanupRepository;
use crate::support::{MemoryRepository, lock};

pub struct Authority {
    pub runs: Arc<MemoryRepository>,
    pub cleanup: Arc<CleanupRepository>,
    pub acknowledged: AtomicBool,
    pub checks: AtomicUsize,
    pub omit_bootstrap: AtomicBool,
    pub log: Arc<std::sync::Mutex<Vec<&'static str>>>,
}
impl Authority {
    pub fn new(fixture: &Fixture) -> Self {
        Self {
            runs: fixture.runs.clone(),
            cleanup: fixture.cleanup.clone(),
            acknowledged: AtomicBool::new(false),
            checks: AtomicUsize::new(0),
            omit_bootstrap: AtomicBool::new(false),
            log: fixture.log.clone(),
        }
    }
    async fn prepared(&self, run: &Run) {
        assert_eq!(run, &self.runs.get(run.id).await.unwrap());
        assert_eq!(run.state, RunState::Provisioning);
    }
}
#[async_trait]
impl RunAuthorityManager for Authority {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunAuthority, RunAuthorityError> {
        self.prepared(run).await;
        lock(&self.log).push("current-authority-prepare");
        Ok(PreparedRunAuthority {
            bootstrap: (!self.omit_bootstrap.load(Ordering::SeqCst)).then(|| {
                RuntimeAuthorityBootstrap::new(
                    run.id.as_uuid(),
                    1,
                    [0x5A; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
                )
            }),
        })
    }
    async fn reauthorize(&self, run: &Run) -> Result<(), RunAuthorityError> {
        assert_eq!(run, &self.runs.get(run.id).await.unwrap());
        self.checks.fetch_add(1, Ordering::SeqCst);
        if self.cleanup.state.lock().await.target.is_some() {
            return Err(RunAuthorityError::redacted(
                "own closure denies execution authority",
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
        self.prepared(run).await;
        assert_eq!((session, generation), (run.id.as_uuid(), 1));
        lock(&self.log).push("current-authority-ack");
        self.acknowledged.store(true, Ordering::SeqCst);
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
impl RunSecretManager for Authority {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        self.prepared(run).await;
        lock(&self.log).push("current-empty-secret-prepare");
        Ok(PreparedRunSecrets::default())
    }
    async fn reauthorize(&self, run: &Run) -> Result<(), RunSecretError> {
        assert_eq!(run, &self.runs.get(run.id).await.unwrap());
        if self.cleanup.state.lock().await.target.is_some() {
            return Err(RunSecretError::redacted(
                "own closure denies empty-secret eligibility",
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
impl RunRuntimeManager for Authority {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        self.prepared(run).await;
        lock(&self.log).push("current-runtime-prepare");
        Ok(PreparedRunRuntime::default())
    }
    async fn destroy(&self, _: RunId) -> Result<(), RunRuntimeError> {
        Ok(())
    }
    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        Ok(0)
    }
}
