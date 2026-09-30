use super::super::ui_request_audit::append_in_transaction as append_ui_request_audit_in_transaction;
use super::PgUiBrowserSessionStore;
use super::audit::{append_handoff_denial, begin_actor_transaction, issue_audit_reason};
use super::rows::{EligibilityInput, IssuedRow};
use forge_domain::OrganizationId;
use release_domain::ui_browser::UiBrowserHandoffId;
use release_service::{
    CreateUiBrowserHandoff, CreatedUiBrowserHandoff, NewUiRequestAuditEvent, UiBrowserHandoffError,
    UiRequestAuditContext, UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason,
    UiRequestAuditSurface,
};
use sqlx::query_as;
use time::Duration;

impl PgUiBrowserSessionStore {
    /// Issues one handoff bound to the exact current published generation.
    ///
    /// # Errors
    ///
    /// Returns [`UiBrowserHandoffError::PermissionDenied`] when the actor,
    /// parent, installation, or published release is no longer eligible.
    /// Returns [`UiBrowserHandoffError::InvalidRoute`] for an undeclared route
    /// and [`UiBrowserHandoffError::Unavailable`] for persistence failures.
    // Keep the complete issuance transaction together so its lock order and
    // post-lock authority checks remain auditable.
    #[allow(clippy::too_many_lines)]
    pub async fn create_ui_browser_handoff(
        &self,
        command: CreateUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserHandoff, UiBrowserHandoffError> {
        let actor_id = command.actor_id;
        let request_id = command.request_id;
        match self.create_ui_browser_handoff_inner(command).await {
            Ok(created) => Ok(created),
            Err(error) => {
                if let Some(reason) = issue_audit_reason(error) {
                    append_handoff_denial(
                        &self.worker_pool,
                        request_id,
                        UiRequestAuditSurface::HandoffIssue,
                        UiRequestAuditContext::actor(actor_id),
                        reason,
                    )
                    .await;
                }
                Err(error)
            }
        }
    }

    async fn create_ui_browser_handoff_inner(
        &self,
        command: CreateUiBrowserHandoff,
    ) -> Result<CreatedUiBrowserHandoff, UiBrowserHandoffError> {
        let mut tx =
            begin_actor_transaction(&self.worker_pool, command.actor_id, command.request_id)
                .await
                .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        let eligible = self
            .lock_and_check_eligibility(
                &mut tx,
                &EligibilityInput {
                    actor_id: command.actor_id,
                    parent_session_id: command.parent_session_id,
                    installation_id: command.installation_id,
                    generation_id: command.generation_id,
                    route: command.route.clone(),
                },
            )
            .await?;
        let handoff_id = UiBrowserHandoffId::new();
        let digest = command.secret.digest().as_bytes();
        let issued = query_as::<_, IssuedRow>(
            r"
            INSERT INTO ui_browser_handoffs (
                id, handoff_digest, request_id, actor_id, parent_session_id,
                installation_id, generation_id, organization_id, route,
                issued_at, expires_at
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9,
                statement_timestamp(), statement_timestamp() + interval '60 seconds'
            ) RETURNING issued_at, expires_at
            ",
        )
        .bind(handoff_id.as_uuid())
        .bind(digest.as_slice())
        .bind(command.request_id.as_uuid())
        .bind(command.actor_id.as_uuid())
        .bind(command.parent_session_id.as_uuid())
        .bind(command.installation_id.as_uuid())
        .bind(command.generation_id.as_uuid())
        .bind(eligible.organization_id)
        .bind(command.route.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        if issued.expires_at != issued.issued_at + Duration::seconds(60) {
            return Err(UiBrowserHandoffError::Unavailable);
        }
        let audit_context = UiRequestAuditContext::verified(
            command.actor_id,
            OrganizationId::from_uuid(eligible.organization_id),
            command.installation_id,
            command.generation_id,
            None,
            None,
        );
        append_ui_request_audit_in_transaction(
            &mut tx,
            NewUiRequestAuditEvent::now(
                command.request_id,
                UiRequestAuditSurface::HandoffIssue,
                UiRequestAuditDecision::Allowed,
                UiRequestAuditOutcome::Succeeded,
                UiRequestAuditReason::None,
                audit_context,
            ),
        )
        .await
        .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        // The published descriptor is locked and checked by the eligibility
        // path above. Record launch intent from that descriptor-derived
        // presentation; callers cannot claim an embed surface themselves.
        if eligible.presentation == "iframe" {
            append_ui_request_audit_in_transaction(
                &mut tx,
                NewUiRequestAuditEvent::now(
                    command.request_id,
                    UiRequestAuditSurface::Embed,
                    UiRequestAuditDecision::Allowed,
                    UiRequestAuditOutcome::Succeeded,
                    UiRequestAuditReason::None,
                    audit_context,
                ),
            )
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        }
        tx.commit()
            .await
            .map_err(|_| UiBrowserHandoffError::Unavailable)?;
        Ok(CreatedUiBrowserHandoff {
            handoff_id,
            actor_id: command.actor_id,
            parent_session_id: command.parent_session_id,
            organization_id: OrganizationId::from_uuid(eligible.organization_id),
            installation_id: command.installation_id,
            generation_id: command.generation_id,
            route: command.route,
            expires_at: issued.expires_at,
        })
    }
}
