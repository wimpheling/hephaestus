use super::PgForgeRepository;
use forge_domain::{ReceiveId, RefUpdate, Repository, RuntimeReceiveProvenance};
use forge_service::{ForgeRepositoryError, ReceiveResult};
use identity_domain::AuthenticatedIdentity;

impl PgForgeRepository {
    /// Records an accepted receive and derives configuration revisions and run
    /// requests in one database transaction.
    ///
    /// The exact new commit of each non-delete update is inspected before the
    /// transaction begins. All resulting audit records, revisions, requests,
    /// and outbox events are then committed atomically.
    ///
    /// # Errors
    ///
    /// Returns an error when Git inspection, serialization, or persistence
    /// fails. No partial database effects are committed.
    pub async fn accept_receive(
        &self,
        repository: &Repository,
        receive_id: ReceiveId,
        principal: &str,
        updates: &[RefUpdate],
    ) -> Result<ReceiveResult, ForgeRepositoryError> {
        self.accept_receive_as(repository, receive_id, principal, None, updates)
            .await
    }

    /// Records an accepted receive with authenticated actor provenance.
    ///
    /// # Errors
    ///
    /// Returns the same failures as [`Self::accept_receive`].
    pub async fn accept_receive_as(
        &self,
        repository: &Repository,
        receive_id: ReceiveId,
        principal: &str,
        identity: Option<&AuthenticatedIdentity>,
        updates: &[RefUpdate],
    ) -> Result<ReceiveResult, ForgeRepositoryError> {
        self.accept_receive_inner(repository, receive_id, principal, identity, updates, None)
            .await
    }

    /// Records an accepted receive authenticated by one exact runtime Git
    /// authority session.
    ///
    /// The optional originating attachment is resolved from the durable
    /// runtime session inside the same receive transaction. The caller cannot
    /// supply an attachment or use a principal string to influence trigger
    /// routing.
    ///
    /// # Errors
    ///
    /// Returns an error when the runtime session cannot be resolved to the
    /// received repository and immutable authority snapshot, or when receive
    /// persistence fails.
    pub async fn accept_runtime_receive(
        &self,
        repository: &Repository,
        receive_id: ReceiveId,
        provenance: RuntimeReceiveProvenance,
        updates: &[RefUpdate],
    ) -> Result<ReceiveResult, ForgeRepositoryError> {
        let principal = format!("runtime:{}", provenance.runtime_session_id);
        self.accept_receive_inner(
            repository,
            receive_id,
            &principal,
            None,
            updates,
            Some(provenance),
        )
        .await
    }
}
