//! `PostgreSQL` gateway management operations.

use super::{
    ConfigureCommandRow, ConfigureGatewayRequest, ConfigureGatewayResult, ConfigureRevisionRow,
    ConfigureRouteRow, GatewayConfigureError, GatewayManagementError, PostgresGatewayManagement,
    begin_actor_transaction, configure_payload_hash, secret_selection_hash,
    validate_secret_selections,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::AuthenticatedIdentity;
use release_domain::{ParameterDeclaration, ParameterDocument, ReleaseCommandKey};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

impl PostgresGatewayManagement {
    /// Validates runtime values and explicit secret selections against the
    /// published release, then atomically creates and activates an immutable
    /// configured revision. Existing mailbox and secret grants are never
    /// copied; a new revision therefore remains fail-closed until callers
    /// explicitly create its bindings.
    ///
    /// # Errors
    ///
    /// Returns authorization, stale-target, bounded-input, idempotency, or
    /// persistence failures without exposing secret metadata.
    #[allow(clippy::too_many_lines)]
    pub async fn configure(
        &self,
        identity: &AuthenticatedIdentity,
        command: ConfigureGatewayRequest,
    ) -> Result<ConfigureGatewayResult, GatewayConfigureError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Gateway, command.gateway_id),
        )
        .await
        .map_err(|error| match error {
            GatewayManagementError::Denied => GatewayConfigureError::Denied,
            GatewayManagementError::Persistence(error) => GatewayConfigureError::Persistence(error),
            _ => GatewayConfigureError::Persistence(sqlx::Error::Protocol(
                "authorization lookup failed".into(),
            )),
        })?;
        let current = sqlx::query_as::<_, ConfigureRevisionRow>(
            "SELECT gateway.project_id, gateway.repository_id, gateway.active_revision_id,
                    gateway.desired_service_revision_id, gateway.lifecycle,
                    revision.release_id, revision.release_agent_id, revision.release_agent_key,
                    revision.handler_contract,
                    revision.service_loopback_port, revision.service_readiness_path,
                    revision.service_health_path, revision.service_log_capture_mode,
                    revision.exposure, revision.secret_slots,
                    revision.mailbox_slots,
                    agent.parameter_schema, release.state AS release_state
             FROM gateways AS gateway
             JOIN gateway_revisions AS revision
               ON revision.gateway_id = gateway.id AND revision.id = $2
             LEFT JOIN release_agents AS agent ON agent.id = revision.release_agent_id
             LEFT JOIN releases AS release ON release.id = revision.release_id
             WHERE gateway.id = $1 AND revision.id = $2",
        )
        .bind(command.gateway_id)
        .bind(command.expected_revision_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayConfigureError::NotFound)?;
        let service_revision = current.handler_contract == "http.service.v1";
        let expected_declared_revision = current
            .desired_service_revision_id
            .or(current.active_revision_id);
        // The durable command ledger is consulted before the active-revision
        // CAS check. A successful retry must replay after another revision
        // becomes active without reactivating its original result.
        let release_id = current.release_id.ok_or(GatewayConfigureError::Stale)?;
        let release_agent_id = current
            .release_agent_id
            .ok_or(GatewayConfigureError::Stale)?;
        let declarations: Vec<ParameterDeclaration> = serde_json::from_value(
            current
                .parameter_schema
                .clone()
                .ok_or(GatewayConfigureError::InvalidArgument)?,
        )
        .map_err(|_| GatewayConfigureError::InvalidArgument)?;
        let parameters = ParameterDocument::resolve(&declarations, &command.parameters)
            .map_err(|_| GatewayConfigureError::InvalidArgument)?;
        let routes = sqlx::query_as::<_, ConfigureRouteRow>(
            "SELECT path, methods, enabled FROM gateway_routes
             WHERE gateway_revision_id = $1 ORDER BY path, id",
        )
        .bind(command.expected_revision_id)
        .fetch_all(&mut *tx)
        .await?;
        validate_secret_selections(
            self,
            &mut tx,
            identity,
            &current,
            &routes,
            &command.secret_selections,
        )
        .await?;
        let payload_hash = configure_payload_hash(
            command.gateway_id,
            command.expected_revision_id,
            release_id,
            release_agent_id,
            parameters.hash().as_bytes(),
            &command.secret_selections,
        );
        let command_key = ReleaseCommandKey::derive(
            "configure_gateway",
            &[identity.idempotency_id.as_uuid().as_bytes()],
        );
        let inserted = sqlx::query(
            "INSERT INTO gateway_configure_commands
               (command_key, operation, gateway_id, expected_revision_id, payload_hash, actor_id, request_id)
             VALUES ($1, 'configure_gateway', $2, $3, $4, $5, $6)
             ON CONFLICT (command_key) DO NOTHING",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(command.gateway_id)
        .bind(command.expected_revision_id)
        .bind(payload_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let prior = sqlx::query_as::<_, ConfigureCommandRow>(
                "SELECT gateway_id, expected_revision_id, payload_hash, actor_id, result_revision_id
                 FROM gateway_configure_commands WHERE command_key = $1",
            )
            .bind(command_key.as_bytes().as_slice())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GatewayConfigureError::Persistence(sqlx::Error::RowNotFound))?;
            if prior.gateway_id != command.gateway_id
                || prior.expected_revision_id != command.expected_revision_id
                || prior.payload_hash != payload_hash.as_slice()
                || prior.actor_id != identity.user_id.as_uuid()
            {
                return Err(GatewayConfigureError::Conflict);
            }
            let revision_id =
                prior
                    .result_revision_id
                    .ok_or(GatewayConfigureError::Persistence(sqlx::Error::Protocol(
                        "incomplete configuration command".into(),
                    )))?;
            tx.commit().await?;
            return Ok(ConfigureGatewayResult { revision_id });
        }
        if expected_declared_revision != Some(command.expected_revision_id) {
            return Err(GatewayConfigureError::Stale);
        }
        if current.lifecycle != "enabled" || current.release_state.as_deref() != Some("published") {
            return Err(GatewayConfigureError::Stale);
        }
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await?;
        // Serialize competing fresh keys on the aggregate before creating a
        // revision. The first winner changes the active revision; followers
        // then observe a clean stale result instead of a uniqueness error.
        let still_declared: bool = sqlx::query_scalar(
            "SELECT lifecycle = 'enabled'
                    AND COALESCE(desired_service_revision_id, active_revision_id) = $2
             FROM gateways WHERE id = $1 FOR UPDATE",
        )
        .bind(command.gateway_id)
        .bind(command.expected_revision_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(false);
        if !still_declared {
            return Err(GatewayConfigureError::Stale);
        }
        // Reinstalling a declaration may select a previously configured
        // predecessor again. A fresh command must create a fresh authority
        // scope without borrowing the old revision's grants. Command replay
        // is resolved by the ledger above, using the stable payload hash.
        let mut revision_digest = Sha256::new();
        revision_digest.update(b"hephaestus.gateway.configured-revision.v1\0");
        revision_digest.update(payload_hash);
        revision_digest.update(command_key.as_bytes());
        let revision_hash: [u8; 32] = revision_digest.finalize().into();
        let revision_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO gateway_revisions
               (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
                release_agent_key, handler_contract, exposure, parameters, secret_slots,
                mailbox_slots, service_loopback_port, service_readiness_path,
                service_health_path, service_log_capture_mode, normalized_hash, created_by)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
        )
        .bind(revision_id)
        .bind(command.gateway_id)
        .bind(current.project_id)
        .bind(current.repository_id)
        .bind(release_id)
        .bind(release_agent_id)
        .bind(current.release_agent_key)
        .bind(current.handler_contract)
        .bind(current.exposure)
        .bind(
            serde_json::to_value(parameters.values())
                .map_err(|_| GatewayConfigureError::InvalidArgument)?,
        )
        .bind(&current.secret_slots)
        .bind(&current.mailbox_slots)
        .bind(current.service_loopback_port)
        .bind(&current.service_readiness_path)
        .bind(&current.service_health_path)
        .bind(&current.service_log_capture_mode)
        .bind(revision_hash.as_slice())
        .bind(identity.user_id.as_uuid())
        .execute(&mut *tx)
        .await?;
        let mut cloned_routes = BTreeMap::new();
        for route in routes {
            let new_route = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO gateway_routes (id,gateway_revision_id,gateway_id,project_id,path,methods,enabled)
                 VALUES ($1,$2,$3,$4,$5,$6,$7)",
            )
            .bind(new_route)
            .bind(revision_id)
            .bind(command.gateway_id)
            .bind(current.project_id)
            .bind(&route.path)
            .bind(&route.methods)
            .bind(route.enabled)
            .execute(&mut *tx)
            .await?;
            cloned_routes.insert(route.path, new_route);
        }
        for selection in &command.secret_selections {
            let binding_id = Uuid::new_v4();
            let selection_hash = secret_selection_hash(selection, revision_id);
            sqlx::query(
                "INSERT INTO gateway_secret_bindings
                   (id,gateway_id,gateway_revision_id,import_id,slot_key,secret_version_id,status,normalized_hash)
                 VALUES ($1,$2,$3,$4,$5,$6,'active',$7)",
            )
            .bind(binding_id)
            .bind(command.gateway_id)
            .bind(revision_id)
            .bind(selection.import_id)
            .bind(&selection.slot_key)
            .bind(selection.secret_version_id)
            .bind(selection_hash.as_slice())
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO gateway_brokered_secret_rules
                   (id,binding_id,gateway_revision_id,gateway_route_id,header_name,normalized_hash)
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(Uuid::new_v4())
            .bind(binding_id)
            .bind(revision_id)
            .bind(cloned_routes[&selection.route_path])
            .bind(&selection.header_name)
            .bind(selection_hash.as_slice())
            .execute(&mut *tx)
            .await?;
        }
        let changed = if service_revision {
            sqlx::query(
                "UPDATE gateways SET desired_service_revision_id = $2, updated_at = now()
                 WHERE id = $1
                   AND COALESCE(desired_service_revision_id, active_revision_id) = $3",
            )
            .bind(command.gateway_id)
            .bind(revision_id)
            .bind(command.expected_revision_id)
            .execute(&mut *tx)
            .await?
        } else {
            sqlx::query(
                "UPDATE gateways SET active_revision_id = $2,
                        desired_service_revision_id = NULL, updated_at = now()
                 WHERE id = $1 AND active_revision_id = $3",
            )
            .bind(command.gateway_id)
            .bind(revision_id)
            .bind(command.expected_revision_id)
            .execute(&mut *tx)
            .await?
        };
        if changed.rows_affected() != 1 {
            return Err(GatewayConfigureError::Stale);
        }
        sqlx::query(
            "UPDATE gateway_configure_commands SET result_revision_id = $2, completed_at = now()
             WHERE command_key = $1",
        )
        .bind(command_key.as_bytes().as_slice())
        .bind(revision_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ConfigureGatewayResult { revision_id })
    }
}
