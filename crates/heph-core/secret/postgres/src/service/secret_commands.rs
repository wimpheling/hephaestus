use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Creates a secret and its first encrypted version atomically.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for denial, invalid owner/mode, encryption
    /// failure, idempotency conflict, or database failure.
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              secret_id = %command.secret_id,
              secret_version_id = %command.version_id
          )
      )]
    #[allow(clippy::too_many_lines)]
    pub async fn create(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateSecret,
    ) -> Result<CreatedSecret, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        let (owner_type, owner_id, owner_organization_id, project_id, organization_id) =
            resolve_owner(&mut tx, command.owner)
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanWriteSecretValue,
            ObjectRef::new(owner_type, owner_id),
        )
        .await?;
        if let Some((aggregate_id, secondary_id)) =
            existing_command(&mut tx, command.command_key, "create")
                .await
                .map_err(|_| SecretServiceError::Persistence)?
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(CreatedSecret {
                secret_id: SecretId::from_uuid(aggregate_id),
                version_id: SecretVersionId::from_uuid(
                    secondary_id.ok_or(SecretServiceError::CorruptIdempotencyRecord)?,
                ),
            });
        }
        let modes = normalized_modes(&command.allowed_delivery_modes)?;
        let context = VersionContext {
            owner: command.owner,
            secret_id: command.secret_id,
            version_id: command.version_id,
            sequence: 1,
            media_type: String::from("application/octet-stream"),
        };
        let encrypted = self.encrypted_store.seal(&context, &command.value)?;
        sqlx::query(
            "INSERT INTO secrets
               (id, owner_organization_id, organization_id, project_id, name,
                status, allowed_delivery_modes, active_version_id, created_by)
               VALUES ($1, $2, $3, $4, $5, 'active', $6, $7, $8)",
        )
        .bind(command.secret_id.as_uuid())
        .bind(owner_organization_id)
        .bind(organization_id)
        .bind(project_id)
        .bind(command.name.as_str())
        .bind(&modes)
        .bind(command.version_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        insert_encrypted_version(
            &mut tx,
            command.secret_id,
            1,
            &encrypted,
            identity.user_id.as_uuid(),
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        record_command(
            &mut tx,
            command.command_key,
            "create",
            command.secret_id.as_uuid(),
            Some(command.version_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "write_value",
            "secret.write_value",
            Some(command.secret_id),
            Some(command.version_id),
            None,
            None,
            "created",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(CreatedSecret {
            secret_id: command.secret_id,
            version_id: command.version_id,
        })
    }

    /// Rotates and compare-and-swap activates a new immutable version.
    ///
    /// # Errors
    ///
    /// Fails closed for denial, stale active versions, revoked state,
    /// encryption failure, or database errors.
    // Keeping encryption, version insert, and active-version CAS together
    // makes the transaction's security boundary directly auditable.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              secret_id = %command.secret_id,
              secret_version_id = %command.new_version_id
          )
      )]
    pub async fn rotate(
        &self,
        identity: &AuthenticatedIdentity,
        command: RotateSecret,
    ) -> Result<SecretVersionId, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::Rotate,
            ObjectRef::new(ObjectType::Secret, command.secret_id.as_uuid()),
        )
        .await?;
        if let Some((_aggregate_id, secondary_id)) =
            existing_command(&mut tx, command.command_key, "rotate")
                .await
                .map_err(|_| SecretServiceError::Persistence)?
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(SecretVersionId::from_uuid(
                secondary_id.ok_or(SecretServiceError::CorruptIdempotencyRecord)?,
            ));
        }
        let row: SecretRotationRow = sqlx::query_as(
            "SELECT owner_organization_id, organization_id, project_id,
                      active_version_id,
                      COALESCE((SELECT max(sequence) FROM secret_versions
                                WHERE secret_id = secrets.id), 0) AS sequence
               FROM secrets WHERE id = $1 AND status = 'active'
               FOR UPDATE",
        )
        .bind(command.secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        if row.active_version_id != Some(command.expected_active_version_id.as_uuid()) {
            return Err(SecretServiceError::StaleActiveVersion);
        }
        let sequence = u64::try_from(row.sequence)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(SecretServiceError::VersionSequenceExhausted)?;
        let owner = row.owner()?;
        let context = VersionContext {
            owner,
            secret_id: command.secret_id,
            version_id: command.new_version_id,
            sequence,
            media_type: String::from("application/octet-stream"),
        };
        let encrypted = self.encrypted_store.seal(&context, &command.value)?;
        insert_encrypted_version(
            &mut tx,
            command.secret_id,
            sequence,
            &encrypted,
            identity.user_id.as_uuid(),
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        let changed = sqlx::query(
            "UPDATE secrets SET active_version_id = $2, updated_at = now()
               WHERE id = $1 AND status = 'active' AND active_version_id = $3",
        )
        .bind(command.secret_id.as_uuid())
        .bind(command.new_version_id.as_uuid())
        .bind(command.expected_active_version_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        if changed.rows_affected() != 1 {
            return Err(SecretServiceError::StaleActiveVersion);
        }
        record_command(
            &mut tx,
            command.command_key,
            "rotate",
            command.secret_id.as_uuid(),
            Some(command.new_version_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            &mut tx,
            identity,
            row.owner_organization_id,
            "rotate",
            "secret.rotate",
            Some(command.secret_id),
            Some(command.new_version_id),
            None,
            None,
            "activated",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(command.new_version_id)
    }
}
