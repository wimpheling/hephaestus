//! Provider-neutral secret application service contracts.

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use release_domain::AgentInstanceRevisionId;
use runtime_types::RunId;
use secret_domain::{SecretGrantId, SecretId, SecretImportId, SecretSlotKey, SecretVersionId};

use super::{
    AcceptSecretImport, BindSecret, BrokerAdapter, BrokerRequest, BrokerResponse, CreateSecret,
    CreatedSecret, GrantAndAcceptSecretImport, GrantSecret, ResolveRunSecrets, ResolvedRawSecret,
    RotateSecret, RuntimeSecretAuthority, SecretServiceError,
};

/// Provider-neutral command service implemented by a persistence adapter.
#[allow(missing_docs)]
#[async_trait]
pub trait SecretCommandService: Send + Sync {
    async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateSecret,
    ) -> Result<CreatedSecret, SecretServiceError>;
    async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotateSecret,
    ) -> Result<SecretVersionId, SecretServiceError>;
    async fn grant(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantSecret,
    ) -> Result<SecretGrantId, SecretServiceError>;
    async fn accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        command: AcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError>;
    async fn grant_and_accept(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantAndAcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError>;
    async fn bind(
        &self,
        identity: &AuthenticatedIdentity,
        command: BindSecret,
    ) -> Result<AgentInstanceRevisionId, SecretServiceError>;
    async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError>;
    async fn set_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
        enabled: bool,
    ) -> Result<(), SecretServiceError>;
    async fn purge(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError>;
}

/// Provider-neutral exact dispatch resolver.
#[allow(missing_docs)]
#[async_trait]
pub trait SecretDispatchResolver: Send + Sync {
    async fn resolve_for_dispatch(
        &self,
        identity: &AuthenticatedIdentity,
        command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError>;
}

/// Provider-neutral runtime lease resolver and broker authorizer.
#[allow(missing_docs)]
#[async_trait]
pub trait SecretRuntimeResolver: Send + Sync {
    async fn receive_raw(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        run_id: RunId,
        slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError>;
    async fn use_brokered(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        adapter: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError>;
}
