use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Creates a service with explicit encryption and authorization providers.
    #[must_use]
    pub const fn new(
        pool: PgPool,
        encrypted_store: EncryptedStore<K>,
        authorizer: Arc<PostgresMelangeAuthorizer>,
    ) -> Self {
        Self {
            pool,
            encrypted_store,
            authorizer,
        }
    }

    pub(super) async fn require(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        permission: Permission,
        object: ObjectRef,
    ) -> Result<(), SecretServiceError> {
        let decision = self
            .authorizer
            .check(tx, Subject::User(identity.user_id), permission, object)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        audit_decision(
            tx,
            identity.user_id,
            permission,
            object,
            decision,
            identity.request_id,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        if decision == AuthorizationDecision::Allow {
            Ok(())
        } else {
            // Keep rejected attempts durable while allowing the caller's
            // command transaction to roll back all domain changes.
            let mut audit_tx = begin_actor_transaction(&self.pool, identity)
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            audit_decision(
                &mut audit_tx,
                identity.user_id,
                permission,
                object,
                decision,
                identity.request_id,
            )
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
            audit_tx
                .commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            Err(SecretServiceError::AuthorizationDenied)
        }
    }

    pub(super) async fn require_binding_mode(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        identity: &AuthenticatedIdentity,
        import_id: SecretImportId,
        mode: DeliveryMode,
    ) -> Result<(), SecretServiceError> {
        self.require(
            tx,
            identity,
            match mode {
                DeliveryMode::Raw => Permission::BindRaw,
                DeliveryMode::Brokered => Permission::BindBrokered,
            },
            ObjectRef::new(ObjectType::SecretImport, import_id.as_uuid()),
        )
        .await
    }
}
