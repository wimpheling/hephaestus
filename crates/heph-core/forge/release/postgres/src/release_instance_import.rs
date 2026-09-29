//! Release instance import commands.

use super::{
    AgentInstanceId, AuthenticatedIdentity, ImportAgent, ObjectRef, ObjectType,
    ParameterDeclaration, ParameterDocument, Permission, ReleaseAgentRow, ReleaseService,
    ReleaseServiceError, RuntimePolicy, append_event, append_instance_event,
    begin_actor_transaction, existing_command, insert_revision, policy_from_contract,
    record_command,
};
use serde_json::{Value, json};
use uuid::Uuid;

impl ReleaseService {
    /// Imports a published release export as a project-owned instance and
    /// atomically activates its first immutable revision.
    ///
    /// Required unresolved secret slots make the revision visibly unrunnable;
    /// no secret value or tenant secret identifier is present in the release.
    ///
    /// # Errors
    ///
    /// Fails for either project-side or source-side denial, invalid parameters
    /// or policy, unpublished release, idempotency conflict, or database error.
    // Instance, optional state-volume metadata, and revision must commit as one.
    #[allow(clippy::too_many_lines)]
    #[tracing::instrument(
        skip_all,
        fields(
            actor_id = %identity.user_id,
            request_id = %identity.request_id,
            instance_id = %command.instance_id,
            release_agent_id = %command.release_agent_id,
            project_id = %command.project_id
        )
    )]
    pub async fn import_agent(
        &self,
        identity: &AuthenticatedIdentity,
        command: ImportAgent,
    ) -> Result<AgentInstanceId, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, command.project_id.as_uuid()),
        )
        .await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUse,
            ObjectRef::new(ObjectType::ReleaseAgent, command.release_agent_id.as_uuid()),
        )
        .await?;
        if let Some(id) = existing_command(&mut tx, command.command_key, "import_agent").await? {
            tx.commit().await?;
            return Ok(AgentInstanceId::from_uuid(id.0));
        }
        let release: ReleaseAgentRow = sqlx::query_as(
            "SELECT agent.family_id, agent.parameter_schema,
                    agent.secret_slot_schema, agent.runtime_contract,
                    agent.requires_state, agent.publication_mode
             FROM release_agents AS agent
             JOIN releases ON releases.id = agent.release_id
             WHERE agent.id = $1 AND releases.state = 'published'",
        )
        .bind(command.release_agent_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ReleaseServiceError::Unavailable)?;
        let declarations: Vec<ParameterDeclaration> =
            serde_json::from_value(release.parameter_schema)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(ReleaseServiceError::InvalidParameters)?;
        let release_policy = policy_from_contract(&release.runtime_contract)?;
        let effective_policy = RuntimePolicy::resolve(
            &release_policy,
            &command.selected_policy,
            &command.platform_policy,
        )?;
        let mut diagnostics = release
            .secret_slot_schema
            .as_array()
            .ok_or(ReleaseServiceError::InvalidStoredData)?
            .iter()
            .filter(|slot| slot.get("required").and_then(Value::as_bool) == Some(true))
            .filter_map(|slot| slot.get("key").and_then(Value::as_str))
            .map(|slot| {
                json!({
                    "code": "required_secret_binding_missing",
                    "field": format!("secret_slots.{slot}")
                })
            })
            .collect::<Vec<_>>();
        let required_capability_slots: Vec<String> = sqlx::query_scalar(
            "SELECT slot_key FROM release_capability_requirements
             WHERE release_agent_id = $1 AND slot_required
             ORDER BY slot_key",
        )
        .bind(command.release_agent_id.as_uuid())
        .fetch_all(&mut *tx)
        .await?;
        diagnostics.extend(required_capability_slots.into_iter().map(|slot| {
            json!({
                "code": "required_capability_binding_missing",
                "field": format!("capability_slots.{slot}")
            })
        }));
        let runnable = diagnostics.is_empty();
        let state_volume_id = release.requires_state.then(Uuid::new_v4);
        sqlx::query(
            "INSERT INTO agent_instances
             (id, project_id, family_id, name, state, active_revision_id,
              state_volume_id, created_by)
             VALUES ($1, $2, $3, $4, 'active', NULL, $5, $6)",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.project_id.as_uuid())
        .bind(release.family_id)
        .bind(command.name.as_str())
        .bind(state_volume_id)
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if let Some(volume_id) = state_volume_id {
            sqlx::query(
                "INSERT INTO agent_instance_state_volumes
                 (id, instance_id, state, capacity_bytes)
                 VALUES ($1, $2, 'uninitialized', 1073741824)",
            )
            .bind(volume_id)
            .bind(command.instance_id.as_uuid())
            .execute(&mut *tx)
            .await?;
        }
        insert_revision(
            &mut tx,
            command.revision_id,
            command.instance_id,
            command.release_agent_id,
            &parameters,
            &command.selected_policy,
            &effective_policy,
            &command.platform_policy_version,
            &release.publication_mode,
            None,
            runnable,
            &diagnostics,
            identity,
        )
        .await?;
        sqlx::query(
            "UPDATE agent_instances
             SET active_revision_id = $2, updated_at = now(), version = version + 1
             WHERE id = $1 AND active_revision_id IS NULL",
        )
        .bind(command.instance_id.as_uuid())
        .bind(command.revision_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        record_command(
            &mut tx,
            command.command_key,
            "import_agent",
            command.instance_id.as_uuid(),
            Some(command.revision_id.as_uuid()),
            Some(identity),
        )
        .await?;
        append_instance_event(
            &mut tx,
            command.instance_id,
            Some(command.revision_id),
            "instance.created",
            identity,
            json!({"runnable": runnable}),
        )
        .await?;
        append_event(
            &mut tx,
            command.instance_id.as_uuid(),
            "hephaestus.agent_instance.created.v1",
            "agent_instance.created.v1",
            json!({
                "schema_version": 1,
                "instance_id": command.instance_id,
                "revision_id": command.revision_id,
                "release_agent_id": command.release_agent_id,
                "project_id": command.project_id,
                "runnable": runnable,
            }),
        )
        .await?;
        tx.commit().await?;
        Ok(command.instance_id)
    }
}
