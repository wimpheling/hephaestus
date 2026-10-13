//! Counted delegation to the real provider; secret resolution is forbidden.
use async_trait::async_trait;
use heph_secret::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretMountManager,
    SecretMountProvider, SecretRuntimeError,
};
use identity_domain::AuthenticatedIdentity;
use runtime_types::RunId;
use secret_application::{
    BrokerAdapter, BrokerRequest, BrokerResponse, ResolveRunSecrets, ResolvedRawSecret,
    RuntimeSecretAuthority, SecretDispatchResolver, SecretRuntimeResolver, SecretServiceError,
};
use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey};
use secret_postgres::PostgresSecretMountMetadata;
use secret_runtime::FilesystemSecretMountProvider;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub struct NoSecretEffects;
#[async_trait]
impl SecretDispatchResolver for NoSecretEffects {
    async fn resolve_for_dispatch(
        &self,
        _: &AuthenticatedIdentity,
        _: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        panic!("ordinary cleanup must not dispatch secrets")
    }
}
#[async_trait]
impl SecretRuntimeResolver for NoSecretEffects {
    async fn receive_raw(
        &self,
        _: &OpaqueRuntimeCredential,
        _: RunId,
        _: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        panic!("ordinary cleanup must not decrypt secrets")
    }
    async fn use_brokered(
        &self,
        _: &OpaqueRuntimeCredential,
        _: &BrokerRequest,
        _: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError> {
        panic!("ordinary cleanup must not invoke brokered secrets")
    }
}

#[derive(Default)]
pub struct CountedProvider {
    real: FilesystemSecretMountProvider,
    calls: Mutex<Vec<&'static str>>,
}
impl CountedProvider {
    pub fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, call: &'static str) {
        self.calls.lock().unwrap().push(call);
    }
}
impl SecretMountProvider for CountedProvider {
    fn validate_config(&self, config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError> {
        self.record("validate");
        self.real.validate_config(config)
    }
    fn materialize(
        &self,
        config: &EphemeralSecretConfig,
        run: RunId,
        files: Vec<RawSecretFile>,
        credential: Option<&OpaqueRuntimeCredential>,
    ) -> Result<MaterializedSecretMount, SecretRuntimeError> {
        self.record("materialize");
        self.real.materialize(config, run, files, credential)
    }
    fn discard_materialized(
        &self,
        config: &EphemeralSecretConfig,
        directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        self.record("discard");
        self.real.discard_materialized(config, directory)
    }
    fn destroy_confirmed(
        &self,
        config: &EphemeralSecretConfig,
        directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        self.record("destroy");
        self.real.destroy_confirmed(config, directory)
    }
    fn reconcile_orphans(
        &self,
        config: &EphemeralSecretConfig,
        live: &BTreeSet<String>,
    ) -> Result<usize, SecretRuntimeError> {
        self.record("reconcile");
        self.real.reconcile_orphans(config, live)
    }
}
pub type Manager =
    SecretMountManager<PostgresSecretMountMetadata, NoSecretEffects, NoSecretEffects>;
pub fn manager(
    metadata: PostgresSecretMountMetadata,
    provider: Arc<CountedProvider>,
    config: EphemeralSecretConfig,
) -> Manager {
    SecretMountManager::initialize(metadata, NoSecretEffects, NoSecretEffects, provider, config)
        .unwrap()
}
