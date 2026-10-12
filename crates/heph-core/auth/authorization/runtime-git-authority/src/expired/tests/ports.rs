//! Controlled retained-port forwarding; old methods panic to detect fallback.

use super::*;
use crate::{
    AuthenticatedRuntimeGitAuthority, RuntimeGitCredential, RuntimeGitCredentialHash,
    RuntimeGitCredentialIssuer, RuntimeGitCredentialRepository, RuntimeGitHandoffStore,
};
use async_trait::async_trait;
use git_capability_domain::GitOperation;
use std::{
    future::Future,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

type Request = (Uuid, RuntimeSessionId, RuntimeCredentialGeneration);

fn complete<T>(future: impl Future<Output = T>) -> T {
    let mut future = Box::pin(future);
    let mut context = Context::from_waker(Waker::noop());
    let result = future.as_mut().poll(&mut context);
    drop(future);
    match result {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("controlled Core port unexpectedly yielded"),
    }
}
struct UnsupportedRepository;
#[async_trait]
impl RuntimeGitCredentialRepository for UnsupportedRepository {
    async fn find(
        &self,
        _: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeGitCredential>, RuntimeGitAuthorityError> {
        panic!("legacy find fallback");
    }
    async fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("legacy issuance fallback");
    }
    async fn authenticate(
        &self,
        _: RuntimeGitCredentialHash,
        _: RepositoryId,
        _: GitOperation,
        _: OffsetDateTime,
    ) -> Result<AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError> {
        panic!("live authentication fallback");
    }
}
struct UnsupportedStore;
impl RuntimeGitHandoffStore for UnsupportedStore {
    fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("bearer issuance fallback");
    }
    fn open(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("live open fallback");
    }
    fn destroy(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeGitAuthorityError> {
        panic!("blind destroy fallback");
    }
    fn purge_expired(&self, _: OffsetDateTime) -> Result<u64, RuntimeGitAuthorityError> {
        panic!("global purge fallback");
    }
}

struct ObservingRepository {
    result: Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError>,
    requests: Arc<Mutex<Vec<Request>>>,
}
#[async_trait]
impl RuntimeGitCredentialRepository for ObservingRepository {
    async fn find(
        &self,
        _: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeGitCredential>, RuntimeGitAuthorityError> {
        panic!("legacy find fallback");
    }
    async fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: RuntimeGitCredentialHash,
    ) -> Result<StoredRuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("legacy issuance fallback");
    }
    async fn authenticate(
        &self,
        _: RuntimeGitCredentialHash,
        _: RepositoryId,
        _: GitOperation,
        _: OffsetDateTime,
    ) -> Result<AuthenticatedRuntimeGitAuthority, RuntimeGitAuthorityError> {
        panic!("live authentication fallback");
    }
    async fn observe_expired_run_handoff(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
        self.requests
            .lock()
            .unwrap()
            .push((run, session, generation));
        self.result.clone()
    }
}
struct DeletingStore {
    originals: Arc<Mutex<Vec<ExpiredRuntimeGitHandoff>>>,
    error: Option<RuntimeGitAuthorityError>,
}
impl RuntimeGitHandoffStore for DeletingStore {
    fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("bearer issuance fallback");
    }
    fn open(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        panic!("live open fallback");
    }
    fn destroy(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeGitAuthorityError> {
        panic!("blind destroy fallback");
    }
    fn purge_expired(&self, _: OffsetDateTime) -> Result<u64, RuntimeGitAuthorityError> {
        panic!("global purge fallback");
    }
    fn destroy_expired(
        &self,
        original: &ExpiredRuntimeGitHandoff,
    ) -> Result<(), RuntimeGitAuthorityError> {
        self.originals.lock().unwrap().push(original.clone());
        self.error.map_or(Ok(()), Err)
    }
}

fn request(original: &ExpiredRuntimeGitHandoff) -> Request {
    (
        original.ceiling().run(),
        original.ceiling().session().session,
        original.ceiling().session().generation,
    )
}

#[test]
fn default_ports_are_unsupported_without_legacy_fallback() {
    let original = present(Uuid::from_u128(1));
    let (run, session, generation) = request(&original);
    let issuer = RuntimeGitCredentialIssuer::new(UnsupportedRepository, UnsupportedStore);
    assert_eq!(
        complete(issuer.observe_expired_run_handoff(run, session, generation)).unwrap_err(),
        RuntimeGitAuthorityError::Unsupported
    );
    assert_eq!(
        issuer.destroy_expired_handoff(&original),
        Err(RuntimeGitAuthorityError::Unsupported)
    );
}

#[test]
fn observation_and_deletion_forward_exactly_to_original_retained_ports() {
    let original = present(Uuid::from_u128(1));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let originals = Arc::new(Mutex::new(Vec::new()));
    let issuer = RuntimeGitCredentialIssuer::new(
        ObservingRepository {
            result: Ok(original.clone()),
            requests: Arc::clone(&requests),
        },
        DeletingStore {
            originals: Arc::clone(&originals),
            error: None,
        },
    );
    let (run, session, generation) = request(&original);
    let observed = complete(issuer.observe_expired_run_handoff(run, session, generation)).unwrap();
    assert_eq!(observed, original);
    assert_eq!(*requests.lock().unwrap(), vec![(run, session, generation)]);
    drop(requests);
    assert!(
        originals.lock().unwrap().is_empty(),
        "observation must not delete"
    );
    issuer.destroy_expired_handoff(&observed).unwrap();
    assert_eq!(*originals.lock().unwrap(), vec![original]);
    drop(originals);
    drop(issuer);
}

#[test]
fn adapter_errors_and_mismatched_observations_never_delete_or_fallback() {
    let original = present(Uuid::from_u128(1));
    let (run, session, generation) = request(&original);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let originals = Arc::new(Mutex::new(Vec::new()));
    for result in [
        Err(RuntimeGitAuthorityError::Persistence),
        Ok(original.clone()),
    ] {
        let expected = if result.is_err() {
            RuntimeGitAuthorityError::Persistence
        } else {
            RuntimeGitAuthorityError::IdentityMismatch
        };
        let issuer = RuntimeGitCredentialIssuer::new(
            ObservingRepository {
                result,
                requests: Arc::clone(&requests),
            },
            DeletingStore {
                originals: Arc::clone(&originals),
                error: None,
            },
        );
        let requested_run = if expected == RuntimeGitAuthorityError::Persistence {
            run
        } else {
            Uuid::from_u128(99)
        };
        assert_eq!(
            complete(issuer.observe_expired_run_handoff(requested_run, session, generation))
                .unwrap_err(),
            expected
        );
        drop(issuer);
    }
    drop(requests);
    assert!(originals.lock().unwrap().is_empty());
    let issuer = RuntimeGitCredentialIssuer::new(
        UnsupportedRepository,
        DeletingStore {
            originals: Arc::clone(&originals),
            error: Some(RuntimeGitAuthorityError::HandoffUnavailable),
        },
    );
    assert_eq!(
        issuer.destroy_expired_handoff(&original),
        Err(RuntimeGitAuthorityError::HandoffUnavailable)
    );
    assert_eq!(*originals.lock().unwrap(), vec![original]);
    drop(originals);
    drop(issuer);
}
