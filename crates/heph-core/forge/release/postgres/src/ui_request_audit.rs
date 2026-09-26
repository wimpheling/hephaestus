//! Worker-owned persistence for redacted UI request audit events.

use async_trait::async_trait;
use release_service::{NewUiRequestAuditEvent, UiRequestAuditError, UiRequestAuditSink};
use sqlx::{PgPool, Postgres, Transaction};

/// Worker-role `PostgreSQL` sink for UI request audit events.
#[derive(Clone)]
pub struct PgUiRequestAuditRepository {
    worker_pool: PgPool,
}

impl PgUiRequestAuditRepository {
    /// Creates a sink over the explicitly worker-owned pool.
    #[must_use]
    pub const fn new(worker_pool: PgPool) -> Self {
        Self { worker_pool }
    }
}

#[async_trait]
impl UiRequestAuditSink for PgUiRequestAuditRepository {
    async fn append(&self, event: NewUiRequestAuditEvent) -> Result<(), UiRequestAuditError> {
        let mut transaction = self
            .worker_pool
            .begin()
            .await
            .map_err(|_| UiRequestAuditError::Unavailable)?;
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *transaction)
            .await
            .map_err(|_| UiRequestAuditError::Unavailable)?;
        append_in_transaction(&mut transaction, event).await?;
        transaction
            .commit()
            .await
            .map_err(|_| UiRequestAuditError::Unavailable)
    }
}

/// Appends one event to an existing worker transaction.
///
/// The caller must already have selected `hephaestus_worker`. This helper is
/// used by successful handoff issue/exchange transactions so audit evidence
/// commits atomically with the durable handoff mutation. Denials that roll
/// back their protected transaction use [`UiRequestAuditSink::append`] after
/// rollback, preserving independent rejection evidence.
///
/// # Errors
///
/// Returns [`UiRequestAuditError::Unavailable`] when `PostgreSQL` rejects the
/// append or the event violates the migration's closed constraints.
pub async fn append_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    event: NewUiRequestAuditEvent,
) -> Result<(), UiRequestAuditError> {
    let context = event.context();
    sqlx::query(
        "INSERT INTO ui_request_audit_events
            (id, request_id, surface, decision, outcome, reason_code,
             actor_id, organization_id, installation_id, generation_id,
             child_session_id, gateway_id, gateway_revision_id, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(event.id())
    .bind(event.request_id().as_uuid())
    .bind(event.surface().as_str())
    .bind(event.decision().as_str())
    .bind(event.outcome().as_str())
    .bind(event.reason().as_str())
    .bind(context.actor_id().map(identity_domain::UserId::as_uuid))
    .bind(
        context
            .organization_id()
            .map(forge_domain::OrganizationId::as_uuid),
    )
    .bind(
        context
            .installation_id()
            .map(release_domain::UiInstallationId::as_uuid),
    )
    .bind(
        context
            .generation_id()
            .map(release_domain::UiInstallationGenerationId::as_uuid),
    )
    .bind(
        context
            .child_session_id()
            .map(release_domain::ui_browser::UiBrowserSessionId::as_uuid),
    )
    .bind(context.gateway_id().map(gateway_domain::GatewayId::as_uuid))
    .bind(
        context
            .gateway_revision_id()
            .map(gateway_domain::GatewayRevisionId::as_uuid),
    )
    .bind(event.occurred_at())
    .execute(&mut **transaction)
    .await
    .map_err(|_| UiRequestAuditError::Unavailable)?;
    Ok(())
}
