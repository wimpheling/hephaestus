use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Declares one exact immutable outbound HTTPS placeholder rule for an
    /// existing brokered binding.
    #[allow(clippy::too_many_lines)] // The single transaction deliberately keeps authority and rule creation atomic.
    pub async fn declare_brokered_https_rule(
        &self,
        identity: &AuthenticatedIdentity,
        command: DeclareBrokeredHttpsRule,
    ) -> Result<uuid::Uuid, SecretServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        let binding: BrokeredRuleBindingRow = sqlx::query_as(
            "SELECT revision.instance_id, binding.instance_revision_id, binding.import_id,
                    binding.delivery_mode, binding.destinations, imported.secret_id
               FROM agent_secret_bindings AS binding
               JOIN agent_instance_revisions AS revision
                 ON revision.id = binding.instance_revision_id
               JOIN secret_imports AS imported ON imported.id = binding.import_id
              WHERE binding.id = $1 AND binding.status = 'active'",
        )
        .bind(command.binding_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, binding.instance_id),
        )
        .await?;
        self.require_binding_mode(
            &mut tx,
            identity,
            SecretImportId::from_uuid(binding.import_id),
            DeliveryMode::Brokered,
        )
        .await?;
        if binding.delivery_mode != "brokered" {
            return Err(SecretServiceError::BindingPolicyMismatch);
        }
        if let Some((aggregate_id, _)) =
            existing_command(&mut tx, command.command_key, "declare_brokered_https_rule")
                .await
                .map_err(|_| SecretServiceError::Persistence)?
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(aggregate_id);
        }
        let destination = ExactHttpsOrigin::parse(command.destination)
            .map_err(|_| SecretServiceError::BindingPolicyMismatch)?;
        let host = destination
            .as_str()
            .strip_prefix("https://")
            .filter(|value| !value.contains(':'))
            .ok_or(SecretServiceError::BindingPolicyMismatch)?;
        if !binding.destinations.iter().any(|value| value == host) {
            return Err(SecretServiceError::BindingPolicyMismatch);
        }
        let location = match command.header_prefix {
            Some(prefix) => HttpInjectionLocation::OutboundHeaderPrefix {
                header: BrokeredHeaderName::parse(command.header)
                    .map_err(|_| SecretServiceError::BindingPolicyMismatch)?,
                prefix,
            },
            None => HttpInjectionLocation::OutboundHeaderValue {
                header: BrokeredHeaderName::parse(command.header)
                    .map_err(|_| SecretServiceError::BindingPolicyMismatch)?,
            },
        };
        let secret_version_id: uuid::Uuid = sqlx::query_scalar(
            "SELECT active_version_id FROM secrets WHERE id = $1 AND status = 'active'",
        )
        .bind(binding.secret_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        let rule = BrokeredSecretRule {
            id: BrokeredSecretRuleId::from_uuid(command.rule_id),
            binding_id: command.binding_id.as_uuid(),
            instance_revision_id: binding.instance_revision_id,
            secret_version_id,
            destination: Some(destination),
            location,
            gateway_route_id: None,
        }
        .normalized()
        .map_err(|_| SecretServiceError::BindingPolicyMismatch)?;
        let origin = rule
            .destination
            .as_ref()
            .ok_or(SecretServiceError::BindingPolicyMismatch)?
            .as_str()
            .to_owned();
        let (location_kind, header_name, header_prefix) = match &rule.location {
            HttpInjectionLocation::OutboundHeaderValue { header } => {
                ("outbound_header_value", header.as_str(), None)
            }
            HttpInjectionLocation::OutboundHeaderPrefix { header, prefix } => (
                "outbound_header_prefix",
                header.as_str(),
                Some(prefix.as_str()),
            ),
            HttpInjectionLocation::InboundGatewayHeader { .. } => {
                return Err(SecretServiceError::BindingPolicyMismatch);
            }
        };
        sqlx::query(
            "INSERT INTO brokered_secret_rules
               (id, binding_id, instance_revision_id, secret_version_id, direction,
                destination_origin, location_kind, header_name, header_prefix,
                normalized_hash)
             VALUES ($1, $2, $3, $4, 'outbound', $5, $6, $7, $8, $9)",
        )
        .bind(rule.id.as_uuid())
        .bind(rule.binding_id)
        .bind(rule.instance_revision_id)
        .bind(rule.secret_version_id)
        .bind(origin)
        .bind(location_kind)
        .bind(header_name)
        .bind(header_prefix)
        .bind(rule.normalized_hash().as_slice())
        .execute(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        sqlx::query(
            "SELECT event_id
               FROM append_application_event(
                    $1, 'agent_instance', $2, 'agent_secret_binding', $3,
                    'agent_secret_binding.changed', 'updated', 'active', $2, $4
               )",
        )
        .bind(identity.idempotency_id.as_uuid())
        .bind(binding.instance_id)
        .bind(command.binding_id.as_uuid())
        .bind(binding.import_id)
        .fetch_one(&mut *tx)
        .await
        .map(|_| ())
        .map_err(|_| SecretServiceError::Persistence)?;
        record_command(
            &mut tx,
            command.command_key,
            "declare_brokered_https_rule",
            command.rule_id,
            Some(command.binding_id.as_uuid()),
            identity,
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(command.rule_id)
    }

    /// Enables or disables later secret resolution without changing versions.
    ///
    /// Disabling is immediate for new leases and requires revocation
    /// authority. Re-enabling requires rotation authority and does not restore
    /// any separately revoked grant, import, binding, session, or lease.
    ///
    /// # Errors
    ///
    /// Fails for denial, invalid lifecycle, idempotency conflict, or database
    /// failure.
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              %secret_id,
              enabled
          )
      )]
    pub async fn set_secret_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        command_key: SecretCommandKey,
        secret_id: SecretId,
        enabled: bool,
    ) -> Result<(), SecretServiceError> {
        let operation = if enabled { "enable" } else { "disable" };
        let permission = if enabled {
            Permission::Rotate
        } else {
            Permission::Revoke
        };
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            permission,
            ObjectRef::new(ObjectType::Secret, secret_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command_key, operation)
            .await
            .map_err(|_| SecretServiceError::Persistence)?
            .is_some()
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(());
        }
        let expected = if enabled { "disabled" } else { "active" };
        let next = if enabled { "active" } else { "disabled" };
        let owner_organization_id: Uuid = sqlx::query_scalar(
            "UPDATE secrets SET status = $2, updated_at = now()
               WHERE id = $1 AND status = $3
               RETURNING owner_organization_id",
        )
        .bind(secret_id.as_uuid())
        .bind(next)
        .bind(expected)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| SecretServiceError::Persistence)?
        .ok_or(SecretServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command_key,
            operation,
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
            operation,
            permission.as_str(),
            Some(secret_id),
            None,
            None,
            None,
            if enabled {
                "later_resolution_enabled"
            } else {
                "later_resolution_disabled"
            },
        )
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(())
    }
}
