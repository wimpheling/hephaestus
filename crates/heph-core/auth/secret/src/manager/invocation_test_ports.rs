//! Controlled ports only; these tests establish no worker/119 or physical proof.

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use run_domain::Run;
use run_orchestrator::RunSecretError;
use runtime_types::RunId;
use secret_application::{
    BrokerAdapter, BrokerRequest, BrokerResponse, ResolveRunSecrets, ResolvedRawSecret,
    RuntimeSecretAuthority, SecretDispatchResolver, SecretRuntimeResolver, SecretServiceError,
};
use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey};
use uuid::Uuid;

use crate::{
    EphemeralSecretConfig, MaterializedSecretMount, RawSecretFile, SecretDispatchInput,
    SecretMountManager, SecretMountMetadata, SecretMountProvider, SecretRuntimeError,
};

// Every historical metadata operation panics. The Invocation path may invoke
// only its dedicated verifier; a fallback would fail the controlled test.

pub struct DefaultMetadata;

#[async_trait]
impl SecretMountMetadata for DefaultMetadata {
    async fn dispatch_input(
        &self,
        _run: &Run,
    ) -> Result<Option<SecretDispatchInput>, RunSecretError> {
        panic!("historical dispatch/identity path must not run")
    }
    async fn persist_mount(&self, _run: RunId, _directory: Uuid) -> Result<(), RunSecretError> {
        panic!("Invocation must not persist a secret mount")
    }
    async fn authorized(&self, _run: &Run) -> Result<bool, RunSecretError> {
        panic!("historical secret leases must not authorize Invocation")
    }
    async fn materialized_directory(&self, _run: RunId) -> Result<Option<Uuid>, RunSecretError> {
        panic!("no materialized secret directory expected")
    }
    async fn mark_destroyed(&self, _run: RunId) -> Result<(), RunSecretError> {
        panic!("no secret mount to destroy")
    }
    async fn live_directories(&self) -> Result<BTreeSet<String>, RunSecretError> {
        panic!("no secret reconciliation expected")
    }
    async fn mark_cleaned_mounts_destroyed(&self) -> Result<(), RunSecretError> {
        panic!("no secret cleanup mutation expected")
    }
}

pub struct QualifiedMetadata {
    pub saved: Run,
    pub saved_bindings: Vec<Uuid>,
    pub calls: Arc<AtomicUsize>,
}

#[async_trait]
impl SecretMountMetadata for QualifiedMetadata {
    async fn verify_qualified_invocation_empty_bindings(
        &self,
        run: &Run,
    ) -> Result<(), RunSecretError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // A controlled stand-in for a future real worker, not production proof.
        if run != &self.saved || !self.saved_bindings.is_empty() {
            return Err(RunSecretError::redacted(
                "controlled saved evidence differs",
            ));
        }
        Ok(())
    }
    async fn dispatch_input(
        &self,
        _run: &Run,
    ) -> Result<Option<SecretDispatchInput>, RunSecretError> {
        panic!("historical dispatch/identity path must not run")
    }
    async fn persist_mount(&self, _run: RunId, _directory: Uuid) -> Result<(), RunSecretError> {
        panic!("Invocation must not persist a secret mount")
    }
    async fn authorized(&self, _run: &Run) -> Result<bool, RunSecretError> {
        panic!("historical secret leases must not authorize Invocation")
    }
    async fn materialized_directory(&self, _run: RunId) -> Result<Option<Uuid>, RunSecretError> {
        panic!("no materialized secret directory expected")
    }
    async fn mark_destroyed(&self, _run: RunId) -> Result<(), RunSecretError> {
        panic!("no secret mount to destroy")
    }
    async fn live_directories(&self) -> Result<BTreeSet<String>, RunSecretError> {
        panic!("no secret reconciliation expected")
    }
    async fn mark_cleaned_mounts_destroyed(&self) -> Result<(), RunSecretError> {
        panic!("no secret cleanup mutation expected")
    }
}

pub struct NoHistoricalSecrets;

#[async_trait]
impl SecretDispatchResolver for NoHistoricalSecrets {
    async fn resolve_for_dispatch(
        &self,
        _identity: &AuthenticatedIdentity,
        _command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        panic!("must not dispatch using a reconstructed historical identity")
    }
}

#[async_trait]
impl SecretRuntimeResolver for NoHistoricalSecrets {
    async fn receive_raw(
        &self,
        _credential: &OpaqueRuntimeCredential,
        _run: RunId,
        _slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        panic!("no raw secret may be read")
    }
    async fn use_brokered(
        &self,
        _credential: &OpaqueRuntimeCredential,
        _request: &BrokerRequest,
        _adapter: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError> {
        panic!("no brokered secret operation expected")
    }
}

struct NoMountIo;

impl SecretMountProvider for NoMountIo {
    fn validate_config(&self, _config: &EphemeralSecretConfig) -> Result<(), SecretRuntimeError> {
        Ok(())
    }
    fn materialize(
        &self,
        _config: &EphemeralSecretConfig,
        _run: RunId,
        _files: Vec<RawSecretFile>,
        _credential: Option<&OpaqueRuntimeCredential>,
    ) -> Result<MaterializedSecretMount, SecretRuntimeError> {
        panic!("must not create a physical secret mount")
    }
    fn discard_materialized(
        &self,
        _config: &EphemeralSecretConfig,
        _directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        panic!("no physical secret mount exists")
    }
    fn destroy_confirmed(
        &self,
        _config: &EphemeralSecretConfig,
        _directory: Uuid,
    ) -> Result<(), SecretRuntimeError> {
        panic!("no physical secret cleanup expected")
    }
    fn reconcile_orphans(
        &self,
        _config: &EphemeralSecretConfig,
        _live: &BTreeSet<String>,
    ) -> Result<usize, SecretRuntimeError> {
        panic!("no physical orphan reconciliation expected")
    }
}

pub fn manager<M: SecretMountMetadata + 'static>(
    metadata: M,
) -> SecretMountManager<M, NoHistoricalSecrets, NoHistoricalSecrets> {
    SecretMountManager::initialize(
        metadata,
        NoHistoricalSecrets,
        NoHistoricalSecrets,
        Arc::new(NoMountIo),
        EphemeralSecretConfig {
            root: PathBuf::from("/controlled-unused-secret-root"),
            require_memory_filesystem: true,
        },
    )
    .unwrap()
}
