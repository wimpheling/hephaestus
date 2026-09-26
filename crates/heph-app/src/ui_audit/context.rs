use axum::http::Extensions;
use identity_domain::RequestId;
use release_service::{
    UiBrowserHandoffError, UiBrowserSessionContext, UiRequestAuditContext, UiRequestAuditReason,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAppendFailure {
    Timeout,
    SinkUnavailable,
}

impl AuditAppendFailure {
    pub const fn error_class(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::SinkUnavailable => "sink-unavailable",
        }
    }
}

/// Correlation allocated once by the outer UI middleware and read by inner
/// handlers. It carries no authority and is never serialized.
#[derive(Clone, Copy)]
pub struct UiRequestCorrelation(pub RequestId);

/// Reads the outer correlation. The fallback exists only for direct unit
/// router calls; the production composition always inserts the extension
/// before either handler runs.
pub fn correlation_id(extensions: &Extensions) -> RequestId {
    extensions
        .get::<UiRequestCorrelation>()
        .map_or_else(RequestId::new, |correlation| correlation.0)
}

/// Converts a verified safe child context into the audit context. The parent
/// session identity is intentionally not copied because it is not a safe
/// serving reference in the public audit type.
pub const fn verified_context(context: &UiBrowserSessionContext) -> UiRequestAuditContext {
    UiRequestAuditContext::verified(
        context.actor_id,
        context.organization_id,
        context.installation_id,
        context.generation_id,
        Some(context.session_id),
        None,
    )
}

pub const fn reason_for_content_error(
    error: crate::ui_browser_content::UiContentError,
) -> UiRequestAuditReason {
    use crate::ui_browser_content::UiContentError;
    match error {
        UiContentError::InvalidRequest | UiContentError::BodyTooLarge => {
            UiRequestAuditReason::InvalidInput
        }
        UiContentError::NotFound => UiRequestAuditReason::NotFound,
        UiContentError::Unauthenticated => UiRequestAuditReason::Unauthenticated,
        UiContentError::RangeNotSatisfiable => UiRequestAuditReason::InvalidInput,
        UiContentError::GuestSetCookie | UiContentError::GuestRedirect => {
            UiRequestAuditReason::UpstreamFailure
        }
        UiContentError::Unavailable => UiRequestAuditReason::Unavailable,
    }
}

pub const fn reason_for_bootstrap_error(error: UiBrowserHandoffError) -> UiRequestAuditReason {
    use UiBrowserHandoffError;
    match error {
        UiBrowserHandoffError::PermissionDenied => UiRequestAuditReason::Unauthorized,
        UiBrowserHandoffError::InvalidRoute => UiRequestAuditReason::InvalidRoute,
        UiBrowserHandoffError::InvalidOrExpired => UiRequestAuditReason::Expired,
        UiBrowserHandoffError::Unavailable => UiRequestAuditReason::Unavailable,
    }
}
