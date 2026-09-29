//! Release instance attachment commands.

use super::{
    AgentAttachmentId, AgentInstanceId, AuthenticatedIdentity, CreateAttachment, ObjectRef,
    ObjectType, Permission, ReleaseService, ReleaseServiceError, RemoveAttachment,
    SetAttachmentEnabled, append_event, append_instance_event, begin_actor_transaction,
    existing_command, record_command, ref_selector_string, trigger_policy_name,
};
use serde_json::json;
use uuid::Uuid;

impl ReleaseService {
    /// Creates an exact project-bound attachment. Trigger policy lives on this
    /// record, not on the release's source branch.
    ///
    /// # Errors
    ///
    /// Fails for either instance or repository denial, cross-project target,
    /// idempotency conflict, or database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id,
            instance_id = %command.instance_id,
            repository_id = %command.repository_id
        )
    )]
    pub async fn create_attachment(
        &self,
        identity: &AuthenticatedIdentity,
        command: CreateAttachment,
    ) -> Result<AgentAttachmentId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, command.instance_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanWrite,
            ObjectRef::new(ObjectType::Repository, command.repository_id.as_uuid()),
        )
        .await?;
        if let Some(id) =
            existing_command(&mut tx, command.command_key, "create_attachment").await?
        {
            tx.commit().await?;
            return Ok(AgentAttachmentId::from_uuid(id.0));
        }
        let project_id: Uuid =
            sqlx::query_scalar("SELECT project_id FROM agent_instances WHERE id = $1")
                .bind(command.instance_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ReleaseServiceError::Unavailable)?;
        sqlx::query(
            "INSERT INTO agent_attachments
             (id, instance_id, project_id, repository_id, ref_selector,
              trigger_policy, enabled, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, true, $7)",
        )
        .bind(command.attachment_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(project_id)
        .bind(command.repository_id.as_uuid())
        .bind(ref_selector_string(&command.ref_selector))
        .bind(trigger_policy_name(command.trigger_policy))
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "create_attachment",
            command.attachment_id.as_uuid(),
            Some(command.instance_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            None,
            "attachment.created",
            identity,
            json!({
                "attachment_id": command.attachment_id,
                "repository_id": command.repository_id,
            }),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": command.instance_id,
                "attachment_id": command.attachment_id,
                "repository_id": command.repository_id,
                "action": "created",
                "enabled": true,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.attachment_id)
    }

    /// Enables or disables future triggers for an attachment.
    ///
    /// # Errors
    ///
    /// Fails for denial, a removed/missing attachment, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id
        )
    )]
    pub async fn set_attachment_enabled(
        &self,
        identity: &AuthenticatedIdentity,
        command: SetAttachmentEnabled,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentAttachment, command.attachment_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "set_attachment_enabled")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let instance_id: Uuid = sqlx::query_scalar(
            "UPDATE agent_attachments
             SET enabled = $2, updated_at = now()
             WHERE id = $1 AND removed_at IS NULL
             RETURNING instance_id",
        )
        .bind(command.attachment_id.as_uuid())
        .bind(command.enabled)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command.command_key,
            "set_attachment_enabled",
            command.attachment_id.as_uuid(),
            Some(instance_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(instance_id),
            None,
            if command.enabled {
                "attachment.enabled"
            } else {
                "attachment.disabled"
            },
            identity,
            json!({"attachment_id": command.attachment_id}),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": instance_id,
                "attachment_id": command.attachment_id,
                "action": if command.enabled { "enabled" } else { "disabled" },
                "enabled": command.enabled,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Tombstones an attachment while preserving historical run references.
    ///
    /// # Errors
    ///
    /// Fails for denial, a missing attachment, idempotency conflict, or
    /// database failure.
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            attachment_id = %command.attachment_id
        )
    )]
    pub async fn remove_attachment(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveAttachment,
    ) -> Result<(), ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentAttachment, command.attachment_id.as_uuid()),
        )
        .await?;
        if existing_command(&mut tx, command.command_key, "remove_attachment")
            .await?
            .is_some()
        {
            tx.commit().await?;
            return Ok(());
        }
        let instance_id: Uuid = sqlx::query_scalar(
            "UPDATE agent_attachments
             SET enabled = false, removed_at = COALESCE(removed_at, now()),
                 updated_at = now()
             WHERE id = $1
             RETURNING instance_id",
        )
        .bind(command.attachment_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        record_command(
            &mut tx,
            command.command_key,
            "remove_attachment",
            command.attachment_id.as_uuid(),
            Some(instance_id),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            AgentInstanceId::from_uuid(instance_id),
            None,
            "attachment.removed",
            identity,
            json!({"attachment_id": command.attachment_id}),
        )
        .await?;
        append_event(
            &mut tx,
            command.attachment_id.as_uuid(),
            "hephaestus.agent_instance.attachment_changed.v1",
            "agent_instance.attachment_changed.v1",
            json!({
                "instance_id": instance_id,
                "attachment_id": command.attachment_id,
                "action": "removed",
                "enabled": false,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
