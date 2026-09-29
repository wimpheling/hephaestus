use super::*;

impl<K: KeyProvider + Send + Sync> SecretService<K> {
    /// Binds an opaque import to a declared slot by creating and CAS-activating
    /// a new immutable instance revision. Existing bindings are cloned to new
    /// revision-bound identities; no historical binding is rewritten.
    ///
    /// # Errors
    ///
    /// Fails unless the actor can configure the instance and bind every
    /// carried import in its exact mode, the source grant/import/secret remain
    /// active, attachment scope matches, the release declaration accepts the
    /// request, and the active revision wins compare-and-swap.
    #[tracing::instrument(
          skip_all,
          fields(
              actor_id = %identity.user_id,
              request_id = %identity.request_id,
              binding_id = %command.binding_id,
              instance_id = %command.instance_id,
              revision_id = %command.new_revision_id,
              import_id = %command.import_id
          )
      )]
    pub async fn bind_secret(
        &self,
        identity: &AuthenticatedIdentity,
        mut command: BindSecret,
    ) -> Result<AgentInstanceRevisionId, SecretServiceError> {
        command.phases.sort_unstable_by_key(|phase| *phase as u8);
        command.phases.dedup();
        command.attachment_ids.sort_unstable();
        command.attachment_ids.dedup();
        command.destinations.sort_unstable();
        command.destinations.dedup();
        if command.phases.is_empty() || command.destinations.len() > 32 {
            return Err(SecretServiceError::BindingPolicyMismatch);
        }
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require_binding_mode(&mut tx, identity, command.import_id, command.mode)
            .await?;
        if let Some((_binding_id, revision_id)) =
            existing_command(&mut tx, command.command_key, "bind")
                .await
                .map_err(|_| SecretServiceError::Persistence)?
        {
            tx.commit()
                .await
                .map_err(|_| SecretServiceError::Persistence)?;
            return Ok(AgentInstanceRevisionId::from_uuid(
                revision_id.ok_or(SecretServiceError::CorruptIdempotencyRecord)?,
            ));
        }
        let plan = self
            .prepare_binding_revision(&mut tx, identity, &command)
            .await?;
        self.persist_binding_revision(&mut tx, identity, &command, plan)
            .await?;
        tx.commit()
            .await
            .map_err(|_| SecretServiceError::Persistence)?;
        Ok(command.new_revision_id)
    }
}
