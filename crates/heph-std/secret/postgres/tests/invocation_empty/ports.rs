//! Controlled effect ports around a real `PostgreSQL` metadata verifier.

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

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
use uuid::Uuid;

pub struct NoSecretEffects;

#[async_trait]
impl SecretDispatchResolver for NoSecretEffects {
    async fn resolve_for_dispatch(
        &self,
        _identity: &AuthenticatedIdentity,
        _command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        panic!("saved-empty Invocation must not reconstruct or dispatch historical identity")
    }
}
#[async_trait]
impl SecretRuntimeResolver for NoSecretEffects {
    async fn receive_raw(
        &self,
        _credential: &OpaqueRuntimeCredential,
        _run: RunId,
        _slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        panic!("saved-empty Invocation must not decrypt raw secrets")
    }
    async fn use_brokered(
        &self,
        _credential: &OpaqueRuntimeCredential,
        _request: &BrokerRequest,
        _adapter: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError> {
        panic!("saved-empty Invocation must not dispatch brokered secrets")
    }
}
struct NoMountIo;
impl SecretMountProvider for NoMountIo {
    fn validate_config(&self, _config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError> {
        // The controlled unused root tests only effect avoidance. Actual Local
        // volume custody belongs to the reused genuine B1 fixture.
        Ok(())
    }
    fn materialize(
        &self,
        _config: &EphemeralSecretConfig,
        _run: RunId,
        _files: Vec<RawSecretFile>,
        _credential: Option<&OpaqueRuntimeCredential>,
    ) -> Result<MaterializedSecretMount, SecretRuntimeError> {
        panic!("saved-empty Invocation must not materialize a mount")
    }
    fn discard_materialized(
        &self,
        _config: &EphemeralSecretConfig,
        _directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        panic!("saved-empty Invocation has no materialization to discard")
    }
    fn destroy_confirmed(
        &self,
        _config: &EphemeralSecretConfig,
        _directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        panic!("saved-empty verification must not destroy a mount")
    }
    fn reconcile_orphans(
        &self,
        _config: &EphemeralSecretConfig,
        _live: &BTreeSet<String>,
    ) -> Result<usize, SecretRuntimeError> {
        panic!("saved-empty verification must not reconcile mounts")
    }
}

pub type Manager =
    SecretMountManager<PostgresSecretMountMetadata, NoSecretEffects, NoSecretEffects>;

pub fn manager(metadata: PostgresSecretMountMetadata) -> Manager {
    SecretMountManager::initialize(
        metadata,
        NoSecretEffects,
        NoSecretEffects,
        Arc::new(NoMountIo),
        EphemeralSecretConfig {
            root: PathBuf::from("/controlled-unused-secret-root"),
            require_memory_filesystem: true,
        },
    )
    .unwrap()
}
