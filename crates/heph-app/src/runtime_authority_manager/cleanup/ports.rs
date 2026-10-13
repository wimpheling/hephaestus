//! Controlled retained Core ports. Every old/bulk fallback panics.

use async_trait::async_trait;
use capability_domain::{RuntimeCredential, RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_authority::{
    ExpiredRuntimeHandoff, NewRuntimeSession, RuntimeAuthorityError, RuntimeHandoffStore,
    RuntimeSessionRepository, StoredRuntimeSession,
};
use runtime_git_authority::{
    AuthenticatedRuntimeGitAuthority, ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError,
    RuntimeGitCredential, RuntimeGitCredentialHash, RuntimeGitCredentialRepository,
    RuntimeGitHandoffStore, StoredRuntimeGitCredential,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct Ports {
    pub(super) log: Arc<Mutex<Vec<&'static str>>>,
    pub(super) generic: Result<ExpiredRuntimeHandoff, RuntimeAuthorityError>,
    pub(super) git: Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError>,
    pub(super) fail_generic: Arc<AtomicBool>,
    pub(super) fail_git: Arc<AtomicBool>,
    pub(super) deleted_generic: Arc<Mutex<Vec<ExpiredRuntimeHandoff>>>,
    pub(super) deleted_git: Arc<Mutex<Vec<ExpiredRuntimeGitHandoff>>>,
}
impl Ports {
    pub(super) fn new(generic: ExpiredRuntimeHandoff, git: ExpiredRuntimeGitHandoff) -> Self {
        Self {
            log: Arc::new(Mutex::new(vec![])),
            generic: Ok(generic),
            git: Ok(git),
            fail_generic: Arc::new(AtomicBool::new(false)),
            fail_git: Arc::new(AtomicBool::new(false)),
            deleted_generic: Arc::new(Mutex::new(vec![])),
            deleted_git: Arc::new(Mutex::new(vec![])),
        }
    }
}
#[async_trait]
impl RuntimeSessionRepository for Ports {
    async fn find(
        &self,
        _: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        panic!("find fallback")
    }
    async fn create(
        &self,
        _: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("issue fallback")
    }
    async fn acknowledge(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("ack fallback")
    }
    async fn revoke(
        &self,
        _: RuntimeSessionId,
        _: OffsetDateTime,
        _: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("revoke fallback")
    }
    async fn expire(&self, _: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        panic!("global expire fallback")
    }
    async fn observe_expired_run_handoff(
        &self,
        _: Uuid,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeHandoff, RuntimeAuthorityError> {
        self.log.lock().unwrap().push("observe generic");
        self.generic.clone()
    }
}
impl RuntimeHandoffStore for Ports {
    fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        panic!("create fallback")
    }
    fn open(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        panic!("live open fallback")
    }
    fn destroy(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        panic!("blind destroy fallback")
    }
    fn purge_expired(&self, _: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        panic!("global purge fallback")
    }
    fn destroy_expired(
        &self,
        original: &ExpiredRuntimeHandoff,
    ) -> Result<(), RuntimeAuthorityError> {
        self.log.lock().unwrap().push("delete generic");
        self.deleted_generic.lock().unwrap().push(original.clone());
        if self.fail_generic.load(Ordering::SeqCst) {
            Err(RuntimeAuthorityError::HandoffUnavailable)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl RuntimeGitCredentialRepository for Ports {
    async fn find(
        &self,
        _: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeGitCredential>, RuntimeGitAuthorityError> {
        panic!("git find fallback")
    }
    async fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("git issue fallback")
    }
    async fn authenticate(
        &self,
        _: RuntimeGitCredentialHash,
        _: git_capability_domain::RepositoryId,
        _: git_capability_domain::GitOperation,
        _: OffsetDateTime,
    ) -> Result<AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError> {
        panic!("live git auth fallback")
    }
    async fn observe_expired_run_handoff(
        &self,
        _: Uuid,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
        self.log.lock().unwrap().push("observe git");
        self.git.clone()
    }
}
impl RuntimeGitHandoffStore for Ports {
    fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("git create fallback")
    }
    fn open(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("live git open fallback")
    }
    fn destroy(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeGitAuthorityError> {
        panic!("blind git destroy fallback")
    }
    fn purge_expired(&self, _: OffsetDateTime) -> Result<u64, RuntimeGitAuthorityError> {
        panic!("global git purge fallback")
    }
    fn destroy_expired(
        &self,
        original: &ExpiredRuntimeGitHandoff,
    ) -> Result<(), RuntimeGitAuthorityError> {
        self.log.lock().unwrap().push("delete git");
        self.deleted_git.lock().unwrap().push(original.clone());
        if self.fail_git.load(Ordering::SeqCst) {
            Err(RuntimeGitAuthorityError::HandoffUnavailable)
        } else {
            Ok(())
        }
    }
}
