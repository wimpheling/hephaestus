use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use mailbox_domain::MailboxId;
use runtime_types::AgentInstanceId;
use uuid::Uuid;

use crate::{
    MailboxPersistenceError, PostgresMailboxRepository, errors::storage,
    helpers::allocation_command_key,
};
impl PostgresMailboxRepository {
    /// Allocates the one mailbox owned by an agent instance.
    ///
    /// Project and instance management authority is checked before the
    /// worker-only mailbox write. The command ledger makes retries return the
    /// original mailbox even after another operation changes the instance.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxPersistenceError::Unavailable`] for an inaccessible
    /// instance and [`MailboxPersistenceError::IdempotencyConflict`] when a
    /// retry key is reused for another instance.
    pub async fn allocate(
        &self,
        identity: &AuthenticatedIdentity,
        instance_id: AgentInstanceId,
    ) -> Result<MailboxId, MailboxPersistenceError> {
        let key = allocation_command_key(identity);
        let mut transaction = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(storage)?;
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        let project_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT project_id FROM agent_instances
             WHERE id = $1
               AND state <> 'removed'
               AND check_permission('user', hephaestus_actor_id(), 'can_manage',
                    'agent_instance', id::text) = 1
               AND check_permission('user', hephaestus_actor_id(), 'can_manage',
                    'project', project_id::text) = 1",
        )
        .bind(instance_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let project_id = project_id.ok_or(MailboxPersistenceError::Unavailable)?;
        let inserted = sqlx::query(
            "INSERT INTO mailbox_allocation_commands
                 (command_key, operation, project_id, instance_id, actor_id,
                  mailbox_id, request_id)
             VALUES ($1, 'create_mailbox', $2, $3, $4, $5, $6)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(key.as_slice())
        .bind(project_id)
        .bind(instance_id.as_uuid())
        .bind(identity.user_id.as_uuid())
        .bind(Option::<Uuid>::None)
        .bind(identity.request_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if inserted.rows_affected() == 0 {
            let stored: Option<(String, Uuid, Uuid, Uuid, Option<Uuid>)> = sqlx::query_as(
                "SELECT operation, project_id, instance_id, actor_id, mailbox_id
                 FROM mailbox_allocation_commands WHERE command_key = $1",
            )
            .bind(key.as_slice())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(storage)?;
            let Some((operation, stored_project, stored_instance, stored_actor, mailbox_id)) =
                stored
            else {
                return Err(MailboxPersistenceError::Unavailable);
            };
            if operation != "create_mailbox"
                || stored_project != project_id
                || stored_instance != instance_id.as_uuid()
                || stored_actor != identity.user_id.as_uuid()
            {
                return Err(MailboxPersistenceError::IdempotencyConflict);
            }
            let mailbox_id = mailbox_id.ok_or(MailboxPersistenceError::Unavailable)?;
            transaction.commit().await.map_err(storage)?;
            return Ok(MailboxId::from_uuid(mailbox_id));
        }
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        // The mailbox trigger emits the instance-scoped product event only
        // for this user allocation command. Worker bootstrap and recovery
        // writes must not manufacture mutation receipts.
        sqlx::query("SET LOCAL hephaestus.mailbox_allocation = 'true'")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        let mailbox_id: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO mailboxes (id, project_id, instance_id, state)
             VALUES ($1, $2, $3, 'active')
             ON CONFLICT (instance_id) DO UPDATE SET updated_at = now()
             WHERE mailboxes.state <> 'removed'
             RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(project_id)
        .bind(instance_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let mailbox_id = mailbox_id.ok_or(MailboxPersistenceError::Unavailable)?;
        sqlx::query(
            "UPDATE mailbox_allocation_commands SET mailbox_id = $2
             WHERE command_key = $1",
        )
        .bind(key.as_slice())
        .bind(mailbox_id)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        Ok(MailboxId::from_uuid(mailbox_id))
    }

    /// Ensures the supplied mailbox is owned by exactly one project instance.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxPersistenceError`] when the durable ownership boundary
    /// rejects the requested mailbox.
    pub async fn ensure_mailbox(
        &self,
        project_id: Uuid,
        mailbox_id: MailboxId,
        instance_id: AgentInstanceId,
    ) -> Result<(), MailboxPersistenceError> {
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
        // A JetStream dispatch command remains stable across redelivery, but a
        // retry is a distinct logical VM execution.  The run table enforces a
        // globally unique command ID, so bind that command to the deterministic
        // attempt identity rather than the stable dispatch-command identity.
        sqlx::query(
            "INSERT INTO mailboxes (id, project_id, instance_id, state)
             VALUES ($1, $2, $3, 'active')
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(mailbox_id.as_uuid())
        .bind(project_id)
        .bind(instance_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        let owned: bool = sqlx::query_scalar(
            "SELECT project_id = $2 AND instance_id = $3
             FROM mailboxes WHERE id = $1",
        )
        .bind(mailbox_id.as_uuid())
        .bind(project_id)
        .bind(instance_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .unwrap_or(false);
        if owned {
            transaction.commit().await.map_err(storage)?;
            Ok(())
        } else {
            transaction.rollback().await.map_err(storage)?;
            Err(MailboxPersistenceError::Unavailable)
        }
    }
}
