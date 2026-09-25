//! `PostgreSQL` gateway management operations.

use super::{
    GatewayMailboxBindingCommandRow, GatewayMailboxBindingRow, GatewayMailboxBindingSummary,
    GatewayMailboxBindingTargetRow, GatewayManagementError, PostgresGatewayManagement,
    begin_actor_transaction, binding_command_key, binding_payload_hash, load_mailbox_binding,
    valid_gateway_producer, valid_gateway_slot,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

impl PostgresGatewayManagement {
    /// Performs a capability-checked lifecycle compare-and-swap. A false
    /// result deliberately represents a stale or already-transitioned state.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization or persistence failure.
    pub async fn transition(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
        expected: &str,
        next: &str,
    ) -> Result<bool, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let changed: bool =
            sqlx::query_scalar("SELECT gateway_transition_lifecycle($1, $2, $3, $4, $5)")
                .bind(gateway_id)
                .bind(expected)
                .bind(next)
                .bind(identity.user_id.as_uuid())
                .bind(identity.request_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// Creates one immutable exact mailbox binding and its active publication
    /// grant after checking authority over both the gateway revision and the
    /// target agent instance.
    ///
    /// # Errors
    ///
    /// Returns an authorization, absence, bounded-input, or persistence error.
    // This transaction deliberately keeps authority, immutable binding, and
    // committed receipt/outbox work together so a grant cannot be published
    // without its corresponding product event.
    #[allow(clippy::too_many_lines)]
    pub async fn create_mailbox_binding(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_revision_id: Uuid,
        slot_key: &str,
        mailbox_id: Uuid,
        producer_id: &str,
    ) -> Result<GatewayMailboxBindingSummary, GatewayManagementError> {
        if !valid_gateway_slot(slot_key) || !valid_gateway_producer(producer_id) {
            return Err(GatewayManagementError::InvalidArgument);
        }
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::GatewayRevision, gateway_revision_id),
        )
        .await?;
        let instance_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT instance_id FROM mailboxes WHERE id = $1 AND state = 'active'",
        )
        .bind(mailbox_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::AgentInstance, instance_id),
        )
        .await?;
        let command_key = binding_command_key(identity, "create_gateway_mailbox_binding");
        let payload_hash = binding_payload_hash(
            "create_gateway_mailbox_binding",
            gateway_revision_id,
            slot_key,
            Some(mailbox_id),
            producer_id,
            None,
        );
        let inserted = sqlx::query(
            "INSERT INTO gateway_mailbox_binding_commands
                (command_key, operation, gateway_revision_id, slot_key, mailbox_id,
                 producer_id, payload_hash, actor_id, request_id)
             VALUES ($1, 'create_gateway_mailbox_binding', $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(gateway_revision_id)
        .bind(slot_key)
        .bind(mailbox_id)
        .bind(producer_id)
        .bind(payload_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let prior = sqlx::query_as::<_, GatewayMailboxBindingCommandRow>(
                "SELECT operation, gateway_revision_id, slot_key, mailbox_id,
                        producer_id, target_binding_id, payload_hash, actor_id,
                        result_binding_id
                 FROM gateway_mailbox_binding_commands WHERE command_key = $1",
            )
            .bind(command_key.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GatewayManagementError::Unavailable)?;
            if prior.operation != "create_gateway_mailbox_binding"
                || prior.gateway_revision_id != gateway_revision_id
                || prior.slot_key.as_deref() != Some(slot_key)
                || prior.mailbox_id != Some(mailbox_id)
                || prior.producer_id.as_deref() != Some(producer_id)
                || prior.target_binding_id.is_some()
                || prior.payload_hash.as_slice() != payload_hash.as_slice()
                || prior.actor_id != identity.user_id.as_uuid()
            {
                return Err(GatewayManagementError::Conflict);
            }
            let binding_id = prior
                .result_binding_id
                .ok_or(GatewayManagementError::Unavailable)?;
            let row = load_mailbox_binding(&mut tx, binding_id)
                .await?
                .ok_or(GatewayManagementError::Unavailable)?;
            tx.commit().await?;
            return Ok(row.into());
        }
        // Binding lifecycle writes remain under the actor-scoped application
        // role. The runtime worker may settle publications, but cannot mint
        // or impersonate an operator's authority grant.
        let binding_id = Uuid::new_v4();
        let grant_id = Uuid::new_v4();
        let row = sqlx::query_as::<_, GatewayMailboxBindingRow>(
            "WITH revision AS (
                 SELECT id, gateway_id, project_id FROM gateway_revisions WHERE id = $1
             ), binding AS (
                 INSERT INTO gateway_mailbox_bindings
                    (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id, producer_id, created_by)
                 SELECT $2, revision.id, revision.gateway_id, revision.project_id, $3, $4, $5, $6
                 FROM revision
                 RETURNING id, gateway_revision_id, mailbox_id, slot_key, producer_id, created_at
             ), new_grant AS (
                 INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
                 SELECT $7, binding.id, 'active', $6 FROM binding
                 RETURNING id, binding_id, status, granted_at, revoked_at
             )
             SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id, binding.slot_key,
                    binding.producer_id, new_grant.id AS grant_id, new_grant.status AS grant_status,
                    binding.created_at, new_grant.granted_at, new_grant.revoked_at
             FROM binding JOIN new_grant ON new_grant.binding_id = binding.id",
        )
        .bind(gateway_revision_id)
        .bind(binding_id)
        .bind(slot_key)
        .bind(mailbox_id)
        .bind(producer_id)
        .bind(identity.user_id.as_uuid())
        .bind(grant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        sqlx::query(
            "UPDATE gateway_mailbox_binding_commands
             SET result_binding_id = $2, completed_at = now()
             WHERE command_key = $1",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(binding_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }

    /// Revokes the active publication grant while retaining immutable binding
    /// and publication provenance for inspection.
    ///
    /// # Errors
    ///
    /// Returns an authorization, absence, stale-state, or persistence error.
    // Revocation retains the immutable publication chain while atomically
    // recording the grant transition and its committed product event.
    #[allow(clippy::too_many_lines)]
    pub async fn revoke_mailbox_binding_grant(
        &self,
        identity: &AuthenticatedIdentity,
        binding_id: Uuid,
    ) -> Result<GatewayMailboxBindingSummary, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let target = sqlx::query_as::<_, GatewayMailboxBindingTargetRow>(
            "SELECT binding.gateway_revision_id, mailbox.instance_id
             FROM gateway_mailbox_bindings binding
             JOIN mailboxes mailbox ON mailbox.id = binding.mailbox_id
             WHERE binding.id = $1",
        )
        .bind(binding_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::GatewayRevision, target.gateway_revision_id),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanGrantAgentCapability,
            ObjectRef::new(ObjectType::AgentInstance, target.instance_id),
        )
        .await?;
        let command_key = binding_command_key(identity, "revoke_gateway_mailbox_binding_grant");
        let payload_hash = binding_payload_hash(
            "revoke_gateway_mailbox_binding_grant",
            target.gateway_revision_id,
            "",
            None,
            "",
            Some(binding_id),
        );
        let inserted = sqlx::query(
            "INSERT INTO gateway_mailbox_binding_commands
                (command_key, operation, gateway_revision_id, target_binding_id,
                 payload_hash, actor_id, request_id)
             VALUES ($1, 'revoke_gateway_mailbox_binding_grant', $2, $3, $4, $5, $6)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(target.gateway_revision_id)
        .bind(binding_id)
        .bind(payload_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let prior = sqlx::query_as::<_, GatewayMailboxBindingCommandRow>(
                "SELECT operation, gateway_revision_id, slot_key, mailbox_id,
                        producer_id, target_binding_id, payload_hash, actor_id,
                        result_binding_id
                 FROM gateway_mailbox_binding_commands WHERE command_key = $1",
            )
            .bind(command_key.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GatewayManagementError::Unavailable)?;
            if prior.operation != "revoke_gateway_mailbox_binding_grant"
                || prior.gateway_revision_id != target.gateway_revision_id
                || prior.target_binding_id != Some(binding_id)
                || prior.slot_key.is_some()
                || prior.mailbox_id.is_some()
                || prior.producer_id.is_some()
                || prior.payload_hash.as_slice() != payload_hash.as_slice()
                || prior.actor_id != identity.user_id.as_uuid()
            {
                return Err(GatewayManagementError::Conflict);
            }
            let result_binding_id = prior
                .result_binding_id
                .ok_or(GatewayManagementError::Unavailable)?;
            let row = load_mailbox_binding(&mut tx, result_binding_id)
                .await?
                .ok_or(GatewayManagementError::Unavailable)?;
            tx.commit().await?;
            return Ok(row.into());
        }
        let row = sqlx::query_as::<_, GatewayMailboxBindingRow>(
            "WITH updated AS (
                 UPDATE gateway_mailbox_binding_grants
                    SET status = 'revoked', revoked_at = now(), revoked_by = $2
                  WHERE binding_id = $1 AND status = 'active'
                 RETURNING id, binding_id, status, granted_at, revoked_at
             )
             SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id, binding.slot_key,
                    binding.producer_id, updated.id AS grant_id, updated.status AS grant_status,
                    binding.created_at, updated.granted_at, updated.revoked_at
             FROM gateway_mailbox_bindings binding JOIN updated ON updated.binding_id = binding.id",
        )
        .bind(binding_id)
        .bind(identity.user_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::Conflict)?;
        sqlx::query(
            "UPDATE gateway_mailbox_binding_commands
             SET result_binding_id = $2, completed_at = now()
             WHERE command_key = $1",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(binding_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.into())
    }
}
