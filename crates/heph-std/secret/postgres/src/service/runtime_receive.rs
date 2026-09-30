use super::*;

impl<K: KeyProvider + Send + Sync> SecretRuntimeService<K> {
    /// Creates an agent-facing service with separate authorization and
    /// ciphertext-resolver connections.
    #[must_use]
    pub const fn new(
        authorization_pool: PgPool,
        resolver_pool: PgPool,
        encrypted_store: EncryptedStore<K>,
        authorizer: Arc<PostgresMelangeAuthorizer>,
    ) -> Self {
        Self {
            authorization_pool,
            resolver_pool,
            encrypted_store,
            authorizer,
            mount_provider: None,
        }
    }

    /// Installs the host filesystem provider used by the run mount manager.
    #[must_use]
    pub fn with_mount_provider(mut self, provider: Arc<dyn SecretMountProvider>) -> Self {
        self.mount_provider = Some(provider);
        self
    }

    pub(crate) fn mount_provider(&self) -> Option<Arc<dyn SecretMountProvider>> {
        self.mount_provider.clone()
    }

    /// Authenticates and resolves one exact raw lease for ephemeral mounting.
    ///
    /// # Errors
    ///
    /// Fails closed for token/run/slot mismatch, expiry, revocation,
    /// authorization denial, lifecycle changes, tampering, or unavailable
    /// encryption keys.
    #[tracing::instrument(skip_all, fields(run_id = %claimed_run_id, slot = slot.as_str()))]
    pub async fn receive_raw(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        claimed_run_id: RunId,
        slot: SecretSlotKey,
    ) -> Result<ResolvedRawSecret, SecretServiceError> {
        let session = self
            .authenticate_session(credential, claimed_run_id)
            .await?;
        let lease = self
            .authorize_runtime_lease(&session, &slot, DeliveryMode::Raw, Permission::ReceiveRaw)
            .await?;
        let (context, encrypted) = load_runtime_version(
            &self.resolver_pool,
            &session,
            &lease,
            DeliveryMode::Raw,
            true,
            "raw_materialized",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let value = self.encrypted_store.resolve(&context, &encrypted)?;
        Ok(ResolvedRawSecret {
            lease_id: SecretLeaseId::from_uuid(lease.lease_id),
            slot,
            value,
        })
    }
    pub(super) async fn authenticate_session(
        &self,
        credential: &secret_domain::OpaqueRuntimeCredential,
        claimed_run_id: RunId,
    ) -> Result<RuntimeSessionRow, SecretServiceError> {
        let hash = credential.storage_hash();
        let session: RuntimeSessionRow = sqlx::query_as(
            "SELECT session_id, run_id, instance_id, instance_revision_id,
                      attachment_id, phase, expires_at
               FROM authenticate_secret_runtime($1)",
        )
        .bind(hash.as_slice())
        .fetch_optional(&self.authorization_pool)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::RuntimeAuthenticationDenied)?;
        if session.run_id != claimed_run_id.as_uuid()
            || session.expires_at <= OffsetDateTime::now_utc()
        {
            return Err(SecretServiceError::RuntimeAuthenticationDenied);
        }
        Ok(session)
    }

    pub(super) async fn authorize_runtime_lease(
        &self,
        session: &RuntimeSessionRow,
        slot: &SecretSlotKey,
        mode: DeliveryMode,
        permission: Permission,
    ) -> Result<RuntimeLeaseAuthorizationRow, SecretServiceError> {
        let run_id = RunId::from_uuid(session.run_id);
        let mut tx = begin_runtime_transaction(&self.authorization_pool, run_id)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        let lease: RuntimeLeaseAuthorizationRow = sqlx::query_as(
            "SELECT lease.id AS lease_id, lease.secret_version_id,
                      lease.destinations
               FROM secret_leases AS lease
               WHERE lease.session_id = $1 AND lease.run_id = $2
                 AND lease.slot_key = $3 AND lease.delivery_mode = $4
                 AND lease.status = 'active' AND lease.expires_at > now()",
        )
        .bind(session.session_id)
        .bind(session.run_id)
        .bind(slot.as_str())
        .bind(mode_name(mode))
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        let decision = self
            .authorizer
            .check(
                &mut tx,
                Subject::Run(run_id),
                permission,
                ObjectRef::new(ObjectType::SecretLease, lease.lease_id),
            )
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        if decision != AuthorizationDecision::Allow {
            return Err(SecretServiceError::AuthorizationDenied);
        }
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(lease)
    }
}
