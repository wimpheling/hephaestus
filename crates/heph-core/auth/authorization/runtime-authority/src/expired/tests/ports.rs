//! Controlled retained-port forwarding; old methods panic to detect fallback.

use super::*;
use crate::{
    NewRuntimeSession, RuntimeHandoffStore, RuntimeSessionIssuer, RuntimeSessionRepository,
};
use async_trait::async_trait;
use capability_domain::RuntimeCredential;
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
impl RuntimeSessionRepository for UnsupportedRepository {
    async fn find(
        &self,
        _: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        panic!("legacy find fallback");
    }
    async fn create(
        &self,
        _: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("legacy issuance fallback");
    }
    async fn acknowledge(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("legacy acknowledgement fallback");
    }
    async fn revoke(
        &self,
        _: RuntimeSessionId,
        _: OffsetDateTime,
        _: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("legacy revoke fallback");
    }
    async fn expire(&self, _: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        panic!("global expiry fallback");
    }
}
struct UnsupportedStore;
impl RuntimeHandoffStore for UnsupportedStore {
    fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        panic!("bearer issuance fallback");
    }
    fn open(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        panic!("live open fallback");
    }
    fn destroy(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        panic!("blind destroy fallback");
    }
    fn purge_expired(&self, _: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        panic!("global purge fallback");
    }
}

struct ObservingRepository {
    result: Result<ExpiredRuntimeHandoff, RuntimeAuthorityError>,
    requests: Arc<Mutex<Vec<Request>>>,
}
#[async_trait]
impl RuntimeSessionRepository for ObservingRepository {
    async fn find(
        &self,
        _: RuntimeSessionId,
    ) -> Result<Option<StoredRuntimeSession>, RuntimeAuthorityError> {
        panic!("legacy find fallback");
    }
    async fn create(
        &self,
        _: NewRuntimeSession<'_>,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("legacy issuance fallback");
    }
    async fn acknowledge(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("legacy acknowledgement fallback");
    }
    async fn revoke(
        &self,
        _: RuntimeSessionId,
        _: OffsetDateTime,
        _: &str,
    ) -> Result<StoredRuntimeSession, RuntimeAuthorityError> {
        panic!("legacy revoke fallback");
    }
    async fn expire(&self, _: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        panic!("global expiry fallback");
    }
    async fn observe_expired_run_handoff(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeHandoff, RuntimeAuthorityError> {
        self.requests
            .lock()
            .unwrap()
            .push((run, session, generation));
        self.result.clone()
    }
}
struct DeletingStore {
    originals: Arc<Mutex<Vec<ExpiredRuntimeHandoff>>>,
    error: Option<RuntimeAuthorityError>,
}
impl RuntimeHandoffStore for DeletingStore {
    fn create(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        panic!("bearer issuance fallback");
    }
    fn open(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
        _: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        panic!("live open fallback");
    }
    fn destroy(
        &self,
        _: RuntimeSessionId,
        _: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        panic!("blind destroy fallback");
    }
    fn purge_expired(&self, _: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        panic!("global purge fallback");
    }
    fn destroy_expired(
        &self,
        original: &ExpiredRuntimeHandoff,
    ) -> Result<(), RuntimeAuthorityError> {
        self.originals.lock().unwrap().push(original.clone());
        self.error.map_or(Ok(()), Err)
    }
}

fn request(original: &ExpiredRuntimeHandoff) -> Request {
    (
        original.pins().run(),
        original.session().id,
        original.session().generation,
    )
}

#[test]
fn default_ports_are_unsupported_without_legacy_fallback() {
    let original = Fixture::new(Uuid::from_u128(1)).projection().unwrap();
    let (run, session, generation) = request(&original);
    let issuer = RuntimeSessionIssuer::new(UnsupportedRepository, UnsupportedStore);
    assert_eq!(
        complete(issuer.observe_expired_run_handoff(run, session, generation)).unwrap_err(),
        RuntimeAuthorityError::Unsupported
    );
    assert_eq!(
        issuer.destroy_expired_handoff(&original),
        Err(RuntimeAuthorityError::Unsupported)
    );
}

#[test]
fn observation_and_deletion_forward_exactly_to_original_retained_ports() {
    let original = Fixture::new(Uuid::from_u128(1)).projection().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let originals = Arc::new(Mutex::new(Vec::new()));
    let issuer = RuntimeSessionIssuer::new(
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
    let original = Fixture::new(Uuid::from_u128(1)).projection().unwrap();
    let (run, session, generation) = request(&original);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let originals = Arc::new(Mutex::new(Vec::new()));
    for result in [
        Err(RuntimeAuthorityError::Persistence),
        Ok(original.clone()),
    ] {
        let expected = if result.is_err() {
            RuntimeAuthorityError::Persistence
        } else {
            RuntimeAuthorityError::IdentityMismatch
        };
        let issuer = RuntimeSessionIssuer::new(
            ObservingRepository {
                result,
                requests: Arc::clone(&requests),
            },
            DeletingStore {
                originals: Arc::clone(&originals),
                error: None,
            },
        );
        let requested_run = if expected == RuntimeAuthorityError::Persistence {
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
    let issuer = RuntimeSessionIssuer::new(
        UnsupportedRepository,
        DeletingStore {
            originals: Arc::clone(&originals),
            error: Some(RuntimeAuthorityError::HandoffUnavailable),
        },
    );
    assert_eq!(
        issuer.destroy_expired_handoff(&original),
        Err(RuntimeAuthorityError::HandoffUnavailable)
    );
    assert_eq!(*originals.lock().unwrap(), vec![original]);
    drop(originals);
    drop(issuer);
}
