use super::super::ui_request_audit::append_in_transaction as append_ui_request_audit_in_transaction;
use super::PgUiBrowserSessionStore;
use super::audit::{append_handoff_denial, issue_audit_reason};
use super::rows::EligibilityInput;
use forge_domain::OrganizationId;
use identity_domain::{RequestId, UserId};
use release_domain::ui_browser::{UiBrowserRoute, UiBrowserSessionId, child_session_expiry};
use release_domain::{UiInstallationGenerationId, UiInstallationId};
use release_service::{
    CreatedUiBrowserSession, ExchangeUiBrowserHandoff, NewUiRequestAuditEvent,
    UiBrowserHandoffError, UiBrowserSessionContext, UiRequestAuditContext, UiRequestAuditDecision,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSurface,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

impl PgUiBrowserSessionStore {
    /// Atomically exchanges one handoff for one child session.
    ///
    /// This is called by the trusted UI-origin handler after it resolves the
    /// exact generation host behind Caddy's namespace forward. Phoenix creates
    /// the handoff secret through its sensitive authenticated RPC boundary;
    /// this method never returns a secret, digest, or cookie value.
    // Keep child insertion and one-time consumption in one auditable
    // transaction after every authority lock and the final timestamp read.
    #[allow(clippy::too_many_lines)]
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHandoffError::InvalidOrExpired`] when the handoff is
    /// consumed, expired, bound to another generation, or no longer eligible.
    /// Returns [`UiBrowserHandoffError::Unavailable`] for persistence failures.
    pub async fn exchange_ui_browser_handoff(
        &self,
        command: ExchangeUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserSession, UiBrowserHandoffError> {
        let request_id = command.request_id;
        match self.exchange_ui_browser_handoff_inner(command).await {
            Ok(created) => Ok(created),
            Err(error) => {
                if let Some(reason) = issue_audit_reason(error) {
                    append_handoff_denial(
                        &self.worker_pool,
                        request_id,
                        UiRequestAuditSurface::HandoffExchange,
                        UiRequestAuditContext::anonymous(),
                        reason,
                    )
                    .await;
                }
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn exchange_ui_browser_handoff_inner(
        &self,
        command: ExchangeUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserSession, UiBrowserHandoffError> {
        let mut tx = begin_exchange_transaction(&self.worker_pool, command.request_id)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        let handoff_digest = command.handoff_secret.digest().as_bytes();
        let handoff = sqlx::query_as::<_, HandoffRow>(
            r"
            SELECT id, actor_id, parent_session_id, installation_id,
                   generation_id, organization_id, route, issued_at, expires_at,
                   consumed_at
            FROM ui_browser_handoffs
            WHERE handoff_digest = $1
            FOR UPDATE
            ",
        )
        .bind(handoff_digest.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?
        .ok_or(UiBrowserHandoffError::InvalidOrExpired)?;
        if handoff.consumed_at.is_some()
            || handoff.generation_id != command.expected_generation_id.as_uuid()
        {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        set_exchange_actor_context(&mut tx, handoff.actor_id, command.request_id)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;

        let route = UiBrowserRoute::parse(&handoff.route)
            .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
        // Reuse the shared eligibility path to preserve user -> parent ->
        // owner -> installation -> source ordering and every current check.
        let eligibility = self
            .lock_and_check_eligibility(
                &mut tx,
                &EligibilityInput {
                    actor_id: UserId::from_uuid(handoff.actor_id),
                    parent_session_id: identity_domain::BrowserSessionId::from_uuid(
                        handoff.parent_session_id,
                    ),
                    installation_id: UiInstallationId::from_uuid(handoff.installation_id),
                    generation_id: release_domain::UiInstallationGenerationId::from_uuid(
                        handoff.generation_id,
                    ),
                    route: route.clone(),
                },
            )
            .await
            .map_err(|error| match error {
                UiBrowserHandoffError::Unavailable => error,
                _ => UiBrowserHandoffError::InvalidOrExpired,
            })?;
        let parent_survives_handoff = eligibility.parent_expires_at > handoff.issued_at;
        if eligibility.organization_id != handoff.organization_id || !parent_survives_handoff {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }

        // This clock is after the handoff, parent, owner, installation, and
        // mutable release locks. It closes the wait/expiry race before issue.
        let issue_now: OffsetDateTime = sqlx::query_scalar(r"SELECT statement_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if handoff.issued_at > issue_now || handoff.expires_at <= issue_now {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        let child_expires_at = child_session_expiry(issue_now, eligibility.parent_expires_at)
            .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
        let session_id = UiBrowserSessionId::new();
        let session_digest = command.child_secret.digest().as_bytes();
        let inserted = sqlx::query_as::<_, ChildRow>(
            r"
            INSERT INTO ui_browser_sessions (
                id, session_digest, request_id, handoff_id, parent_session_id,
                installation_id, generation_id, organization_id, route,
                issued_at, expires_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
            ) RETURNING issued_at, expires_at
            ",
        )
        .bind(session_id.as_uuid())
        .bind(session_digest.as_slice())
        .bind(command.request_id.as_uuid())
        .bind(handoff.id)
        .bind(handoff.parent_session_id)
        .bind(handoff.installation_id)
        .bind(handoff.generation_id)
        .bind(handoff.organization_id)
        .bind(&handoff.route)
        .bind(issue_now)
        .bind(child_expires_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if inserted.issued_at != issue_now || inserted.expires_at != child_expires_at {
            return Err(UiBrowserHandoffError::Unavailable);
        }
        let consumed = sqlx::query(
            r"UPDATE ui_browser_handoffs
               SET consumed_at = $2
               WHERE id = $1 AND consumed_at IS NULL",
        )
        .bind(handoff.id)
        .bind(issue_now)
        .execute(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if consumed.rows_affected() != 1 {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        let audit_context = UiRequestAuditContext::verified(
            UserId::from_uuid(handoff.actor_id),
            OrganizationId::from_uuid(handoff.organization_id),
            UiInstallationId::from_uuid(handoff.installation_id),
            UiInstallationGenerationId::from_uuid(handoff.generation_id),
            Some(session_id),
            None,
        );
        append_ui_request_audit_in_transaction(
            &mut tx,
            NewUiRequestAuditEvent::now(
                command.request_id,
                UiRequestAuditSurface::HandoffExchange,
                UiRequestAuditDecision::Allowed,
                UiRequestAuditOutcome::Succeeded,
                UiRequestAuditReason::None,
                audit_context,
            ),
        )
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        tx.commit()
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        Ok(CreatedUiBrowserSession {
            context: UiBrowserSessionContext {
                session_id,
                parent_session_id: identity_domain::BrowserSessionId::from_uuid(
                    handoff.parent_session_id,
                ),
                actor_id: UserId::from_uuid(handoff.actor_id),
                organization_id: OrganizationId::from_uuid(handoff.organization_id),
                installation_id: UiInstallationId::from_uuid(handoff.installation_id),
                generation_id: release_domain::UiInstallationGenerationId::from_uuid(
                    handoff.generation_id,
                ),
                route,
                expires_at: inserted.expires_at,
            },
        })
    }
}

async fn begin_exchange_transaction(
    pool: &PgPool,
    request_id: RequestId,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r"SELECT set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $1, true)",
    )
    .bind(request_id.as_uuid().to_string())
    .execute(&mut *tx)
    .await?;
    Ok(tx)
}

async fn set_exchange_actor_context(
    tx: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    request_id: RequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r"SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true)",
    )
    .bind(actor_id.to_string())
    .bind(request_id.as_uuid().to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct HandoffRow {
    id: Uuid,
    actor_id: Uuid,
    parent_session_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    organization_id: Uuid,
    route: String,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    consumed_at: Option<OffsetDateTime>,
}

#[derive(Debug, FromRow)]
struct ChildRow {
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}
