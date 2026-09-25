use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Creates one exact, bounded source-side grant.
    ///
    /// # Errors
    ///
    /// Fails for denial, cross-organization target, invalid policy, inactive
    /// secret, idempotency conflict, or database failure.
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              secret_id = %command.secret_id,
              grant_id = %command.grant_id
          )
      )]
    #[allow(clippy::too_many_lines)]
    pub async fn grant(
        &self,
        identity: &AuthenticatedIdentity,
        mut command: GrantSecret,
    ) -> Result<SecretGrantId, SecretServiceError> {
        command.policy = command.policy.normalized()?;
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::ManageGrants,
            ObjectRef::new(ObjectType::Secret, command.secret_id.as_uuid()),
        )
        .await?;
        if let Some((aggregate_id, _)) = existing_command(&mut tx, command.command_key, "grant")
            .await
            .map_err(|_| SecretServiceError::Persistence)?
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(SecretGrantId::from_uuid(aggregate_id));
        }
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "SELECT owner_organization_id FROM secrets
               WHERE id = $1 AND status = 'active'",
        )
        .bind(command.secret_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        let target = resolve_target(&mut tx, command.target)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        if target.organization_id != owner_organization_id {
            return Err(SecretServiceError::CrossOrganization);
        }
        let modes = command
            .policy
            .delivery_modes
            .iter()
            .map(|mode| mode_name(*mode))
            .collect::<Vec<_>>();
        let phases = command
            .policy
            .phases
            .iter()
            .map(|phase| phase_name(*phase))
            .collect::<Vec<_>>();
        sqlx::query(
            "INSERT INTO secret_grants
               (id, secret_id, owner_organization_id, target_kind, target_id,
                target_project_id, delivery_modes, phases, destinations, status,
                expires_at, created_by)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10, $11)",
        )
        .bind(command.grant_id.as_uuid())
        .bind(command.secret_id.as_uuid())
        .bind(owner_organization_id)
        .bind(target.kind)
        .bind(target.id)
        .bind(target.project_id)
        .bind(modes)
        .bind(phases)
        .bind(&command.policy.destinations)
        .bind(command.expires_at)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        record_command(
            &mut tx,
            command.command_key,
            "grant",
            command.grant_id.as_uuid(),
            Some(command.secret_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            &mut tx,
            identity,
            owner_organization_id,
            "manage_grants",
            "secret.manage_grants",
            Some(command.secret_id),
            None,
            Some(command.grant_id),
            None,
            "granted",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(command.grant_id)
    }

    /// Accepts an opaque import without resolving plaintext or ciphertext.
    ///
    /// # Errors
    ///
    /// Fails for target-side denial, inactive/expired grant, target mismatch,
    /// idempotency conflict, or database failure.
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              grant_id = %command.grant_id,
              import_id = %command.import_id
          )
      )]
    pub async fn accept_import(
        &self,
        identity: &AuthenticatedIdentity,
        command: AcceptSecretImport,
    ) -> Result<SecretImportId, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        let target = resolve_target(&mut tx, command.target)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanAcceptSecretImport,
            ObjectRef::new(target.object_type, target.id),
        )
        .await?;
        if let Some((aggregate_id, _)) = existing_command(&mut tx, command.command_key, "accept")
            .await
            .map_err(|_| SecretServiceError::Persistence)?
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(SecretImportId::from_uuid(aggregate_id));
        }
        let grant: GrantAcceptanceRow = sqlx::query_as(
            "SELECT secret_id, owner_organization_id, target_kind, target_id
               FROM secret_grants
               WHERE id = $1 AND status = 'active'
                 AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(command.grant_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        if grant.target_kind != target.kind || grant.target_id != target.id {
            return Err(SecretServiceError::TargetMismatch);
        }
        sqlx::query(
            "INSERT INTO secret_imports
               (id, grant_id, secret_id, target_kind, target_id, alias,
                status, accepted_by)
               VALUES ($1, $2, $3, $4, $5, $6, 'active', $7)",
        )
        .bind(command.import_id.as_uuid())
        .bind(command.grant_id.as_uuid())
        .bind(grant.secret_id)
        .bind(target.kind)
        .bind(target.id)
        .bind(command.alias.as_str())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        record_command(
            &mut tx,
            command.command_key,
            "accept",
            command.import_id.as_uuid(),
            Some(command.grant_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        audit(
            &mut tx,
            identity,
            grant.owner_organization_id,
            "accept_import",
            "secret_import.accept",
            Some(SecretId::from_uuid(grant.secret_id)),
            None,
            Some(command.grant_id),
            Some(command.import_id),
            "accepted",
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(command.import_id)
    }
}
