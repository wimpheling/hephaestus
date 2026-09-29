mod database;
mod inspection;
mod instance;
mod state;

pub use database::{identity, key, pool, role_pool, seed};
pub use inspection::assert_https_inspection;
pub use instance::{seed_attachment, seed_instance, seed_queued_run};
pub use state::LifecycleState;

use forge_domain::{ProjectId, RepositoryId};
use identity_domain::{OrganizationId, UserId};
use secret_application::{
    BrokerAdapter, BrokerAdapterError, BrokerRequest, BrokerResponse, BrokerStatus,
    VerifiedBrokeredHttpsRule,
};
use secret_domain::SecretValue;
use secret_postgres::SecretService;
use secret_store::LocalKeyProvider;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Notify;
use uuid::Uuid;

pub const SENTINEL: &str = "postgres-secret-sentinel-2747dcb8";

pub type TypedGitAuthority = (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    i64,
    i64,
    i32,
    i32,
    bool,
    Vec<u8>,
);
pub type CarriedTypedGitAuthority = (
    Uuid,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    i64,
    i64,
    i32,
    i32,
    bool,
    Vec<u8>,
);
pub type BrokeredRuleReceipt = (
    Uuid,
    Uuid,
    Uuid,
    Uuid,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    i64,
    Uuid,
    Uuid,
);

pub struct Fixture {
    pub owner: UserId,
    pub organization_secret_manager: UserId,
    pub ordinary_member: UserId,
    pub target_manager: UserId,
    pub other_owner: UserId,
    pub organization: OrganizationId,
    pub target_project: ProjectId,
    pub target_repository: RepositoryId,
    pub other_project: ProjectId,
}

pub struct FakeBroker {
    pub observed: AtomicBool,
    pub expected_credential: Vec<u8>,
}

pub struct AcceptingBroker;

pub struct FailingBroker;

pub struct ObservedBroker {
    pub observed: Arc<AtomicBool>,
}

pub struct ObservedVerifiedBroker {
    pub observed: Arc<Mutex<Option<VerifiedBrokeredHttpsRule>>>,
}

pub struct PausingBroker {
    pub entered: Arc<Notify>,
    pub release: Arc<Notify>,
}

#[async_trait::async_trait]
impl BrokerAdapter for PausingBroker {
    async fn invoke(
        &self,
        _credential: &SecretValue,
        _destination: &str,
        _operation: &str,
        _body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{"result":"sanitized"}"#.to_vec(),
        })
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for ObservedBroker {
    async fn invoke(
        &self,
        _credential: &SecretValue,
        _destination: &str,
        _operation: &str,
        _body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        self.observed.store(true, Ordering::SeqCst);
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{"result":"unexpected"}"#.to_vec(),
        })
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for ObservedVerifiedBroker {
    async fn invoke(
        &self,
        _credential: &SecretValue,
        _destination: &str,
        _operation: &str,
        _body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{"result":"unexpected"}"#.to_vec(),
        })
    }

    async fn invoke_verified_https(
        &self,
        _credential: &SecretValue,
        _request: &BrokerRequest,
        rule: &VerifiedBrokeredHttpsRule,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        *self.observed.lock().expect("verified rule lock") = Some(rule.clone());
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{"result":"sanitized"}"#.to_vec(),
        })
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for FailingBroker {
    async fn invoke(
        &self,
        _credential: &SecretValue,
        _destination: &str,
        _operation: &str,
        _body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        Err(BrokerAdapterError::Retryable)
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for AcceptingBroker {
    async fn invoke(
        &self,
        _credential: &SecretValue,
        _destination: &str,
        _operation: &str,
        _body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{\"result\":\"sanitized\"}"#.to_vec(),
        })
    }
}

#[async_trait::async_trait]
impl BrokerAdapter for FakeBroker {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        self.observed.store(
            credential.expose() == self.expected_credential
                && destination == "api.example.test"
                && operation == "complete"
                && body == b"bounded request",
            Ordering::SeqCst,
        );
        Ok(BrokerResponse {
            status: BrokerStatus::Succeeded,
            body: br#"{"result":"sanitized"}"#.to_vec(),
        })
    }
}
