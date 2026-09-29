use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Revokes a secret and all downstream active authority immediately.
    ///
    /// Active raw leases are marked honestly as potentially observed and their
    /// runs are included in the reconciliation outbox event.
    ///
    /// # Errors
    ///
    /// Fails for denial, missing secret, idempotency conflict, or database
    /// failure.
    // The downstream revocations intentionally remain visible as one atomic
    // reconciliation transaction.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              %secret_id
          )
      )]
    pub async fn revoke_secret(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: SecretCommandKey,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::Revoke,
            ObjectRef::new(ObjectType::Secret, secret_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "revoke")
            .await
            .map_err(|_| SecretServiceError::Persistence)?
            .is_some()
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(());
        }
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "UPDATE secrets
               SET status = 'revoked', revoked_at = now(), updated_at = now()
               WHERE id = $1 AND status IN ('active', 'disabled')
               RETURNING owner_organization_id",
        )
        .bind(secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        sqlx::query(
            "UPDATE secret_grants SET status = 'revoked', revoked_at = now()
               WHERE secret_id = $1 AND status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "UPDATE secret_imports SET status = 'revoked', revoked_at = now()
               WHERE secret_id = $1 AND status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "UPDATE agent_secret_bindings AS binding
               SET status = 'revoked', revoked_at = now()
               FROM secret_imports AS imported
               WHERE binding.import_id = imported.id
                 AND imported.secret_id = $1
                 AND binding.status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "UPDATE secret_leases AS lease
               SET status = 'revoked', revoked_at = now(),
                   raw_material_observed =
                       raw_material_observed OR lease.delivery_mode = 'raw'
               FROM secret_versions AS version
               WHERE lease.secret_version_id = version.id
                 AND version.secret_id = $1
                 AND lease.status = 'active'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "UPDATE secret_runtime_sessions AS session
               SET status = 'revoked', revoked_at = now()
               WHERE session.status = 'active'
                 AND EXISTS (
                     SELECT 1
                     FROM secret_leases AS lease
                     JOIN secret_versions AS version
                       ON version.id = lease.secret_version_id
                     WHERE lease.session_id = session.id
                       AND version.secret_id = $1
                 )",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        record_command(
            &mut tx,
            command_key,
            "revoke",
            secret_id.as_uuid(),
            None,
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "revoke",
            "secret.revoke",
            Some(secret_id),
            None,
            None,
            None,
            "revoked_and_reconciliation_requested",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(())
    }

    /// Tombstones a revoked secret and purges all encrypted version material
    /// only when no active lease retains it.
    ///
    /// # Errors
    ///
    /// Fails for denial, active leases, invalid lifecycle, idempotency
    /// conflict, or database failure.
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              %secret_id
          )
      )]
    #[allow(clippy::too_many_lines)]
    pub async fn purge_secret(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: SecretCommandKey,
        secret_id: SecretId,
    ) -> Result<(), SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::Purge,
            ObjectRef::new(ObjectType::Secret, secret_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, "purge")
            .await
            .map_err(|_| SecretServiceError::Persistence)?
            .is_some()
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(());
        }
        let row: (Uuid, String) = sqlx::query_as(
            "SELECT owner_organization_id, status FROM secrets
               WHERE id = $1 FOR UPDATE",
        )
        .bind(secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        if !matches!(row.1.as_str(), "revoked" | "tombstoned") {
            return Err(SecretServiceError::InvalidLifecycle);
        }
        let active_leases: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM secret_leases AS lease
               JOIN secret_versions AS version
                 ON version.id = lease.secret_version_id
               WHERE version.secret_id = $1
                 AND lease.status = 'active' AND lease.expires_at > now()",
        )
        .bind(secret_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        if active_leases != 0 {
            return Err(SecretServiceError::ActiveLeases);
        }
        sqlx::query(
            "UPDATE secrets SET status = 'tombstoned',
                      tombstoned_at = COALESCE(tombstoned_at, now()),
                      active_version_id = NULL, updated_at = now()
               WHERE id = $1",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "UPDATE secret_versions
               SET status = 'purged', data_nonce = NULL, ciphertext = NULL,
                   wrap_nonce = NULL, wrapped_data_key = NULL,
                   associated_data_hash = NULL, purged_at = now()
               WHERE secret_id = $1 AND status <> 'purged'",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "UPDATE secrets SET status = 'purged', purged_at = now(),
                      updated_at = now() WHERE id = $1",
        )
        .bind(secret_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        record_command(
            &mut tx,
            command_key,
            "purge",
            secret_id.as_uuid(),
            None,
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            &mut tx,
            identity,
            row.0,
            "purge",
            "secret.purge",
            Some(secret_id),
            None,
            None,
            None,
            "cryptographic_material_purged",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(())
    }
}
