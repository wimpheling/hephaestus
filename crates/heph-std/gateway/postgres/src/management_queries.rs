//! `PostgreSQL` gateway management operations.

use super::{
    GatewayIngressRow, GatewayIngressSummary, GatewayMailboxBindingRow,
    GatewayMailboxBindingSummary, GatewayMailboxPublicationManagementRow,
    GatewayMailboxPublicationSummary, GatewayManagementError, GatewayManagementRevision,
    GatewayManagementSummary, GatewayPage, GatewayRevisionRow, GatewayRouteRow, GatewaySummaryRow,
    PostgresGatewayManagement, begin_actor_transaction,
};
use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::AuthenticatedIdentity;
use uuid::Uuid;

impl PostgresGatewayManagement {
    /// Lists gateways visible in one project under forced RLS.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization or persistence failure.
    pub async fn list_project(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayManagementSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Project, project_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewaySummaryRow>(
            "SELECT id, project_id, repository_id, name, lifecycle, active_revision_id,
                    desired_service_revision_id, updated_at
             FROM gateways
             WHERE project_id = $1 AND ($2::uuid IS NULL OR id > $2)
             ORDER BY id LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Returns gateway revisions and route intents only if the caller can read
    /// the exact durable gateway; no request bodies, parameters, or secrets are
    /// projected.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization, absence, or persistence failure.
    pub async fn get(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
    ) -> Result<(GatewayManagementSummary, Vec<GatewayManagementRevision>), GatewayManagementError>
    {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let summary = sqlx::query_as::<_, GatewaySummaryRow>(
            "SELECT id, project_id, repository_id, name, lifecycle, active_revision_id,
                    desired_service_revision_id, updated_at
             FROM gateways WHERE id = $1",
        )
        .bind(gateway_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GatewayManagementError::NotFound)?;
        let revisions = sqlx::query_as::<_, GatewayRevisionRow>(
            "SELECT id, release_id, release_agent_id, handler_contract,
                    service_loopback_port, service_readiness_path, service_health_path,
                    service_log_capture_mode,
                    exposure, secret_slots, mailbox_slots, created_at
             FROM gateway_revisions WHERE gateway_id = $1 ORDER BY created_at DESC, id DESC",
        )
        .bind(gateway_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut result = Vec::with_capacity(revisions.len());
        for revision in revisions {
            let service = revision.service_config()?;
            let routes = sqlx::query_as::<_, GatewayRouteRow>(
                "SELECT id, path, methods, enabled FROM gateway_routes
                 WHERE gateway_revision_id = $1 ORDER BY path, id",
            )
            .bind(revision.id)
            .fetch_all(&mut *tx)
            .await?;
            result.push(GatewayManagementRevision {
                id: revision.id,
                release_id: revision.release_id,
                release_agent_id: revision.release_agent_id,
                handler_contract: revision.handler_contract,
                service,
                exposure: revision.exposure,
                secret_slots: revision.secret_slots,
                mailbox_slots: revision.mailbox_slots,
                created_at: revision.created_at,
                routes: routes.into_iter().map(Into::into).collect(),
            });
        }
        tx.commit().await?;
        Ok((summary.into(), result))
    }

    /// Lists recent redacted ingress records after a gateway-level read check.
    ///
    /// # Errors
    ///
    /// Returns a safe authorization or persistence failure.
    pub async fn ingress(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayIngressSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewayIngressRow>(
            "SELECT id, gateway_revision_id, gateway_route_id, outcome, accepted_at, completed_at
             FROM gateway_invocations
             WHERE gateway_id = $1 AND ($2::uuid IS NULL OR id < $2)
             ORDER BY id DESC LIMIT $3",
        )
        .bind(gateway_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Lists redacted exact bindings for one revision visible to the caller.
    ///
    /// # Errors
    ///
    /// Returns an authorization or persistence error.
    pub async fn mailbox_bindings(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_revision_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayMailboxBindingSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::GatewayRevision, gateway_revision_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewayMailboxBindingRow>(
            "SELECT binding.id, binding.gateway_revision_id, binding.mailbox_id, binding.slot_key,
                    binding.producer_id, binding_grant.id AS grant_id, binding_grant.status AS grant_status,
                    binding.created_at, binding_grant.granted_at, binding_grant.revoked_at
             FROM gateway_mailbox_bindings binding
             JOIN gateway_mailbox_binding_grants binding_grant
               ON binding_grant.binding_id = binding.id
             WHERE binding.gateway_revision_id = $1 AND ($2::uuid IS NULL OR binding.id > $2)
             ORDER BY binding.id LIMIT $3",
        )
        .bind(gateway_revision_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Lists joined, redacted publication provenance for a gateway. It traces
    /// each publication through its exact authorization snapshot, delivery,
    /// and most-recent run while excluding payload/body/header/key/runtime-
    /// session and denial details.
    ///
    /// # Errors
    ///
    /// Returns an authorization or persistence error.
    pub async fn mailbox_publications(
        &self,
        identity: &AuthenticatedIdentity,
        gateway_id: Uuid,
        page: GatewayPage,
    ) -> Result<Vec<GatewayMailboxPublicationSummary>, GatewayManagementError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        self.require(
            &mut tx,
            identity,
            Permission::CanRead,
            ObjectRef::new(ObjectType::Gateway, gateway_id),
        )
        .await?;
        let rows = sqlx::query_as::<_, GatewayMailboxPublicationManagementRow>(
            "SELECT publication.id, publication.invocation_id, publication.gateway_revision_id,
                    publication.binding_id, publication.grant_id, publication.mailbox_id,
                    publication.event_id, publication.slot_key, publication.outcome,
                    publication.accepted_at, publication.settled_at,
                    session.snapshot_id AS authorization_snapshot_id,
                    snapshot_binding.ordinal AS snapshot_binding_ordinal,
                    delivery.disposition AS delivery_disposition,
                    delivery.logical_attempt_count AS delivery_attempt_count,
                    delivery.terminal_at AS delivery_terminal_at,
                    attempt.id AS delivery_attempt_id, attempt.run_id,
                    run.state AS run_state, run.outcome AS run_outcome
             FROM gateway_mailbox_publications publication
             JOIN gateway_invocations invocation ON invocation.id = publication.invocation_id
             JOIN gateway_runtime_authority_sessions session
                ON session.id = publication.runtime_session_id
             LEFT JOIN gateway_authorization_snapshot_bindings snapshot_binding
                ON snapshot_binding.snapshot_id = session.snapshot_id
               AND snapshot_binding.binding_id = publication.binding_id
             LEFT JOIN mailbox_deliveries delivery ON delivery.event_id = publication.event_id
             LEFT JOIN LATERAL (
                SELECT id, run_id FROM mailbox_delivery_attempts
                 WHERE event_id = publication.event_id
                 ORDER BY attempt_number DESC, id DESC LIMIT 1
             ) attempt ON true
             LEFT JOIN runs run ON run.id = attempt.run_id
             WHERE invocation.gateway_id = $1 AND ($2::uuid IS NULL OR publication.id < $2)
             ORDER BY publication.id DESC LIMIT $3",
        )
        .bind(gateway_id)
        .bind(page.after)
        .bind(page.limit)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
