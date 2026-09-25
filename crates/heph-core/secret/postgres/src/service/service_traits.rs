use super::*;

#[async_trait]
impl<K: KeyProvider + Send + Sync> SecretCommandService for SecretService<K> {
    async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateSecret,
    ) -> Result<CreatedSecret, SecretServiceError> {
        self.create(identity, command).await
    }
    async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotateSecret,
    ) -> Result<SecretVersionId, SecretServiceError> {
        self.rotate(identity, command).await
    }
    async fn grant(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantSecret,
    ) -> Result<SecretGrantId, SecretServiceError> {
        self.grant(identity, command).await
    }
    async fn accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        command: AcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        self.accept_import(identity, command).await
    }
    async fn grant_and_accept(
        &self,
        identity: &AuthenticatedIdentity,
        command: GrantAndAcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        self.grant_and_accept(identity, command).await
    }
    async fn bind(
        &self,
        identity: &AuthenticatedIdentity,
        command: BindSecret,
    ) -> Result<AgentInstanceRevisionId, SecretServiceError> {
        self.bind(identity, command).await
    }
    async fn revoke(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        self.revoke(identity, secret_id).await
    }
    async fn set_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
        enabled: bool,
    ) -> Result<(), SecretServiceError> {
        self.set_enabled(identity, secret_id, enabled).await
    }
    async fn purge(
        &self,
        identity: &AuthenticatedIdentity,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        self.purge(identity, secret_id).await
    }
}
#[async_trait]
impl<K: KeyProvider + Send + Sync> SecretDispatchResolver for SecretService<K> {
    async fn resolve_for_dispatch(
        &self,
        identity: &AuthenticatedIdentity,
        command: ResolveRunSecrets,
    ) -> Result<RuntimeSecretAuthority, SecretServiceError> {
        self.resolve_for_dispatch(identity, command).await
    }
}
#[async_trait]
impl<K: KeyProvider + Send + Sync> SecretRuntimeResolver for SecretRuntimeService<K> {
    async fn receive_raw(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        run_id: RunId,
        slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        self.receive_raw(credential, run_id, slot).await
    }
    async fn use_brokered(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        request: &BrokerRequest,
        adapter: &dyn BrokerAdapter,
    ) -> Result<BrokerResponse, SecretServiceError> {
        self.use_brokered(credential, request, adapter).await
    }
}
