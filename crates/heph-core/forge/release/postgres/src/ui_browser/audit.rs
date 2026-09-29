use super::super::ui_request_audit::PgUiRequestAuditRepository;
use identity_domain::{RequestId, UserId};
use release_service::{
    NewUiRequestAuditEvent, UiBrowserHandoffError, UiRequestAuditContext, UiRequestAuditDecision,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface,
};
use sqlx::{PgPool, Postgres, Transaction};

pub(super) async fn begin_actor_transaction(
    pool: &PgPool,
    actor_id: UserId,
    request_id: RequestId,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r"SELECT set_config('hephaestus.actor_id', $1, true),
                  set_config('hephaestus.subject_type', 'user', true),
                  set_config('hephaestus.request_id', $2, true)",
    )
    .bind(actor_id.as_uuid().to_string())
    .bind(request_id.as_uuid().to_string())
    .execute(&mut *tx)
    .await?;
    Ok(tx)
}

pub(super) async fn append_handoff_denial(
    worker_pool: &PgPool,
    request_id: RequestId,
    surface: UiRequestAuditSurface,
    context: UiRequestAuditContext,
    reason: UiRequestAuditReason,
) {
    let repository = PgUiRequestAuditRepository::new(worker_pool.clone());
    if let Err(error) = repository
        .append(NewUiRequestAuditEvent::now(
            request_id,
            surface,
            UiRequestAuditDecision::Denied,
            UiRequestAuditOutcome::NotAttempted,
            reason,
            context,
        ))
        .await
    {
        // Audit persistence must never turn an already-determined denial into
        // a different result, but operators still need a safe diagnostic when
        // the independent evidence write is unavailable.
        tracing::warn!(
            request_id = %request_id,
            surface = %surface,
            reason = reason.as_str(),
            error = %error,
            "UI handoff denial audit append failed"
        );
    }
}

pub(super) const fn issue_audit_reason(
    error: UiBrowserHandoffError,
) -> Option<UiRequestAuditReason> {
    match error {
        UiBrowserHandoffError::PermissionDenied => Some(UiRequestAuditReason::Unauthorized),
        UiBrowserHandoffError::InvalidRoute => Some(UiRequestAuditReason::InvalidRoute),
        UiBrowserHandoffError::InvalidOrExpired => Some(UiRequestAuditReason::Expired),
        // Infrastructure failure is not an authorization denial.
        UiBrowserHandoffError::Unavailable => None,
    }
}
